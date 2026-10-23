//! Data-driven scenarios: the standard checks of the new-machine playbook (`docs/new-machine-playbook.md` in the
//! application repository), read from a manifest instead of written as Rust.
//!
//! A manifest (`<machine>.manifest.toml`) names the machine, how to start it, the reference emulator the goldens
//! came from, and a list of scenarios. A scenario runs the real `retroclawd` headless and compares what it left
//! behind (picture, memory, audio, cycle count, saved state) with a **golden** captured from the reference, or
//! with a number the manifest states. Adding a machine adds a manifest, not code.
//!
//! ```toml
//! machine = "zx81"
//! [reference]
//! emulator = "mame"
//! version = "0.272"
//! command = "mame zx81 -video none ..."
//! [files]                     # option key -> file below RC_ROMS
//! rom = "sinclair/roms/zx81/zx81.rom"
//!
//! [[scenario]]
//! id = "S1"
//! kind = "screen"
//! frames = 100
//! golden = "zx81/S1-boot.png"           # below RC_GOLDENS
//! golden_sha256 = "..."                 # every golden is pinned by hash
//! compare = { mode = "similarity", max_mad = 6.0, crop = [0.1, 0.1, 0.9, 0.9], reason = "the reference draws a different border" }
//! ```
//!
//! **Goldens from another emulator are compared by similarity**, not pixel for pixel: the reference draws its own
//! border, scales its picture and picks its own palette, so only the content can agree. `exact` is for goldens this
//! project made itself (regression). A tolerance without a written `reason` is refused, and a scenario that cannot
//! be matched is marked `status = "known-gap"` with a `gap` (reason and evidence), never loosened until it passes.
//! A known gap that starts passing fails the run until the marker is removed, as the hand-written suites do.
//! A screen/memory known gap may omit an uncaptured golden and its hash. It is
//! reported as not run, never compared with a placeholder or counted as a pass.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{Env, Screen};

/// The reference emulator a manifest's goldens were captured from.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub emulator: String,
    pub version: String,
    #[serde(default)]
    pub command: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub machine: String,
    #[serde(default)]
    pub reference: Option<Reference>,
    /// Extra arguments for every run (`--region ntsc`).
    #[serde(default)]
    pub args: Vec<String>,
    /// Option flag -> file below `RC_ROMS` (`rom = "sinclair/roms/zx81/zx81.rom"` becomes `--rom <path>`).
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    #[serde(rename = "scenario")]
    pub scenarios: Vec<Scenario>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// S1, S2, S3: the picture after `frames` (with scripted `actions`, media in `args`).
    Screen,
    /// S3, S7: a memory range after the run, compared with a raw golden.
    Memory,
    /// S4: tone or silence in the audio.
    Audio,
    /// S5: the saved state is deterministic and a reload continues identically.
    State,
    /// S6: cycles per frame.
    Timing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    #[default]
    Pass,
    KnownGap,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gap {
    pub reason: String,
    /// Where the proof is: a diff image, a trace file, a note of what was tried.
    pub evidence: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    Exact,
    Similarity,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Compare {
    pub mode: Mode,
    /// `exact`: how many pixels may differ (default 0).
    #[serde(default)]
    pub max_pixels: usize,
    /// `similarity`: the largest mean absolute difference (0 to 255) of the two pictures reduced to a 64x48 grid.
    #[serde(default)]
    pub max_mad: f64,
    /// `similarity`: the part of each picture that is compared, `[x0, y0, x1, y1]` as fractions of its size.
    #[serde(default)]
    pub crop: Option<[f64; 4]>,
    /// `similarity`: the golden's own crop, when the reference frames the screen differently from us (Caprice32's
    /// 768x540 picture sits 6 lines higher than our 768x576 one); `crop` then applies to our picture only.
    #[serde(default)]
    pub golden_crop: Option<[f64; 4]>,
    /// `similarity`: stretch each reduced picture to the full 0 to 255 range before comparing, so two emulators that draw
    /// the same content in different palettes (the Spectrum's white is 0xCD in one, 0xC0 in another) still agree.
    #[serde(default)]
    pub normalize: bool,
    /// `similarity`: scale each reduced picture so its most common brightness (the paper, the ground the content is drawn
    /// on) becomes 255 and black stays 0, values above 255 clipped. For two emulators that draw the same black-on-paper
    /// content on a slightly different paper grey, where `normalize` would leave the paper difference in place because
    /// the picture also holds brighter whites.
    #[serde(default)]
    pub anchor_ground: bool,
    /// Why this tolerance is right. Required whenever a tolerance is set.
    #[serde(default)]
    pub reason: String,
}

/// What an `audio` scenario expects of the sound between `from_s` and `to_s` seconds.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioExpect {
    #[serde(default)]
    pub from_s: f64,
    pub to_s: f64,
    /// A tone of this frequency must have at least `min_amplitude` in the window.
    #[serde(default)]
    pub tone_hz: Option<f64>,
    /// Accept the tone anywhere within this percentage of `tone_hz` (the strongest sine in that band is measured): two
    /// emulators' clocks and sample clocks differ by a fraction of a percent. Default 0: exactly `tone_hz`.
    #[serde(default)]
    pub tone_tolerance_pct: f64,
    #[serde(default)]
    pub min_amplitude: f64,
    /// The window must be silent: the level (mean removed) stays below this.
    #[serde(default)]
    pub silent_below_rms: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub id: String,
    pub kind: Kind,
    #[serde(default)]
    pub frames: u32,
    /// Extra arguments for this run (`--tape file.tap`, `--tape-speed 4`).
    #[serde(default)]
    pub args: Vec<String>,
    /// Option flag -> file below `RC_ROMS` for this scenario only (`snapshot = "sinclair/software/zx81/hello.p"`).
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    /// `--at FRAME:ACTION[=FILE]` scripted actions (`"10:key_down=KeyA"`).
    #[serde(default)]
    pub actions: Vec<String>,
    /// A file below `RC_GOLDENS`.
    #[serde(default)]
    pub golden: Option<String>,
    /// Lower-case hex SHA-256 of the golden file. Required with `golden`.
    #[serde(default)]
    pub golden_sha256: Option<String>,
    #[serde(default)]
    pub compare: Option<Compare>,
    /// `memory`: first address and length (at most 64K).
    #[serde(default)]
    pub start: Option<u16>,
    #[serde(default)]
    pub len: Option<u16>,
    #[serde(default)]
    pub audio: Option<AudioExpect>,
    /// `state`: further frames run after the reload.
    #[serde(default)]
    pub more_frames: u32,
    /// `timing`: cycles per frame and how far from it is accepted.
    #[serde(default)]
    pub cycles_per_frame: Option<u64>,
    #[serde(default)]
    pub cycles_tolerance: u64,
    #[serde(default)]
    pub status: Status,
    #[serde(default)]
    pub gap: Option<Gap>,
}

/// How a manifest run ended.
#[derive(Default, Debug)]
pub struct ScenarioReport {
    pub passed: Vec<String>,
    pub failed: Vec<(String, String)>,
    /// Known gaps that still fail: id, reason, how it fails now.
    pub gaps: Vec<(String, String, String)>,
    /// Known gaps that pass now: the marker must go.
    pub closed: Vec<String>,
    /// Not run because an input is unavailable here (firmware `--firmware auto` cannot find): id, why.
    pub skipped: Vec<(String, String)>,
}

/// The line retroclawd prints when `--firmware auto` finds no firmware in the cache, on RomM or at its URL
/// (`apps/retroclawd/src/firmware.rs` in the application): nothing to test against, so the scenario is skipped.
const FIRMWARE_UNAVAILABLE: &str = "--firmware auto: no firmware with a matching SHA-1";

/// Part of the detail retroclawd adds when a file was found but its hash is not the one wanted (a changed download, a
/// bad RomM file): that is a failure, never a skip.
const FIRMWARE_WRONG_HASH: &str = "has SHA-1";

impl ScenarioReport {
    pub fn ok(&self) -> bool {
        self.failed.is_empty() && self.closed.is_empty()
    }

    pub fn summary(&self) -> String {
        let mut s = format!(
            "{} passed, {} failed, {} known gaps, {} gaps now closed, {} skipped\n",
            self.passed.len(),
            self.failed.len(),
            self.gaps.len(),
            self.closed.len(),
            self.skipped.len()
        );
        for (id, why) in &self.failed {
            s += &format!("  FAIL   {id}: {why}\n");
        }
        for id in &self.closed {
            s += &format!("  CLOSED {id}: it passes now, remove its status and gap\n");
        }
        for (id, reason, got) in &self.gaps {
            s += &format!("  gap    {id}: {reason}\n           fails now with: {got}\n");
        }
        for (id, why) in &self.skipped {
            s += &format!("  SKIP   {id}: {why}\n");
        }
        s
    }
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest, String> {
        let manifest: Manifest = toml::from_str(text).map_err(|e| e.to_string())?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn load(path: &Path) -> Result<Manifest, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Manifest::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The rules a manifest must keep, checked before anything runs.
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for s in &self.scenarios {
            if !seen.insert(s.id.as_str()) {
                return Err(format!("scenario {} appears twice", s.id));
            }
            if s.golden.is_some() != s.golden_sha256.is_some() {
                return Err(format!(
                    "scenario {}: `golden` and `golden_sha256` go together (every golden is pinned)",
                    s.id
                ));
            }
            if let Some(c) = &s.compare {
                let tolerant = match c.mode {
                    Mode::Exact => c.max_pixels > 0,
                    Mode::Similarity => true,
                };
                if tolerant && c.reason.trim().is_empty() {
                    return Err(format!(
                        "scenario {}: a tolerance needs a `reason` in `compare`",
                        s.id
                    ));
                }
                for (name, crop) in [("crop", c.crop), ("golden_crop", c.golden_crop)] {
                    if let Some(crop) = crop
                        && !(crop[0] < crop[2]
                            && crop[1] < crop[3]
                            && crop.iter().all(|v| (0.0..=1.0).contains(v)))
                    {
                        return Err(format!(
                            "scenario {}: `{name}` must be [x0,y0,x1,y1] within 0..1",
                            s.id
                        ));
                    }
                }
            }
            match (s.status, &s.gap) {
                (Status::KnownGap, None) => {
                    return Err(format!(
                        "scenario {}: a known gap needs `gap` with a reason and evidence",
                        s.id
                    ));
                }
                (Status::Pass, Some(_)) => {
                    return Err(format!(
                        "scenario {}: `gap` is set but the status is pass",
                        s.id
                    ));
                }
                (_, Some(gap))
                    if gap.reason.trim().is_empty() || gap.evidence.trim().is_empty() =>
                {
                    return Err(format!(
                        "scenario {}: a known gap needs a nonempty reason and evidence",
                        s.id
                    ));
                }
                _ => {}
            }
            let uncaptured = s.status == Status::KnownGap && s.golden.is_none();
            match s.kind {
                Kind::Screen if !uncaptured && (s.golden.is_none() || s.compare.is_none()) => {
                    return Err(format!(
                        "scenario {}: a screen needs `golden` and `compare`",
                        s.id
                    ));
                }
                Kind::Memory
                    if (!uncaptured && s.golden.is_none())
                        || s.start.is_none()
                        || s.len.is_none() =>
                {
                    return Err(format!(
                        "scenario {}: memory needs `golden`, `start` and `len`",
                        s.id
                    ));
                }
                Kind::Audio if !uncaptured && s.audio.is_none() => {
                    return Err(format!("scenario {}: audio needs an `audio` table", s.id));
                }
                Kind::Timing if s.cycles_per_frame.is_none() || s.frames == 0 => {
                    return Err(format!(
                        "scenario {}: timing needs `frames` and `cycles_per_frame`",
                        s.id
                    ));
                }
                Kind::State if s.frames == 0 => {
                    return Err(format!("scenario {}: state needs `frames`", s.id));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// Lower-case hex SHA-256.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Reads a golden below `goldens` and checks its pin.
pub fn read_golden(goldens: &Path, rel: &str, pin: &str) -> Result<(PathBuf, Vec<u8>), String> {
    let path = goldens.join(rel);
    let bytes = std::fs::read(&path).map_err(|e| {
        format!(
            "missing golden {} ({e}); fetch it from RomM",
            path.display()
        )
    })?;
    let got = sha256_hex(&bytes);
    if !got.eq_ignore_ascii_case(pin) {
        return Err(format!(
            "golden {} has hash {got}, the manifest pins {pin}",
            path.display()
        ));
    }
    Ok((path, bytes))
}

/// The picture reduced to a `w` x `h` grid of brightness (0 to 255), over the `crop` part of it.
fn luminance_grid(
    screen: &Screen,
    crop: Option<[f64; 4]>,
    w: usize,
    h: usize,
    normalize: bool,
    anchor_ground: bool,
) -> Vec<f64> {
    let [x0, y0, x1, y1] = crop.unwrap_or([0.0, 0.0, 1.0, 1.0]);
    let px0 = (x0 * screen.width as f64) as usize;
    let px1 = ((x1 * screen.width as f64) as usize).clamp(px0 + 1, screen.width);
    let py0 = (y0 * screen.height as f64) as usize;
    let py1 = ((y1 * screen.height as f64) as usize).clamp(py0 + 1, screen.height);
    let (cw, ch) = (px1 - px0, py1 - py0);
    let lum = |x: usize, y: usize| {
        let p = screen.pixel(x, y);
        0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2])
    };
    // The brightness mapping is fitted on the pixels, not on the reduced grid: thin strokes never fill a grid cell, so the
    // grid's darkest cell is not the ink colour, but some pixel always is.
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    let mut counts = std::collections::HashMap::new();
    for y in py0..py1 {
        for x in px0..px1 {
            let v = lum(x, y);
            lo = lo.min(v);
            hi = hi.max(v);
            *counts.entry(v.round() as i64).or_insert(0usize) += 1;
        }
    }
    let ground = counts
        .into_iter()
        .max_by_key(|&(v, n)| (n, v))
        .map_or(255.0, |(v, _)| v as f64)
        .max(1.0);
    let map = |v: f64| {
        let mut v = v;
        if normalize {
            v = if hi > lo {
                (v - lo) * 255.0 / (hi - lo)
            } else {
                0.0
            };
        }
        if anchor_ground {
            // after `normalize` the ground has moved too: refit it the same way
            let g = if normalize && hi > lo {
                (ground - lo) * 255.0 / (hi - lo)
            } else {
                ground
            };
            v = (v * 255.0 / g.max(1.0)).min(255.0);
        }
        v
    };
    let mut grid = vec![0.0; w * h];
    for gy in 0..h {
        let ya = py0 + gy * ch / h;
        let yb = (py0 + (gy + 1) * ch / h).max(ya + 1).min(py1);
        for gx in 0..w {
            let xa = px0 + gx * cw / w;
            let xb = (px0 + (gx + 1) * cw / w).max(xa + 1).min(px1);
            let mut sum = 0.0;
            let mut n = 0.0;
            for y in ya..yb {
                for x in xa..xb {
                    sum += map(lum(x, y));
                    n += 1.0;
                }
            }
            grid[gy * w + gx] = if n > 0.0 { sum / n } else { 0.0 };
        }
    }
    grid
}

/// Compares our picture with a golden. `Err` says how far apart they are.
pub fn compare_screens(ours: &Screen, golden: &Screen, compare: &Compare) -> Result<(), String> {
    match compare.mode {
        Mode::Exact => {
            if (ours.width, ours.height) != (golden.width, golden.height) {
                return Err(format!(
                    "the picture is {}x{}, the golden {}x{}",
                    ours.width, ours.height, golden.width, golden.height
                ));
            }
            let mut differing = 0usize;
            for y in 0..ours.height {
                for x in 0..ours.width {
                    if ours.pixel(x, y) != golden.pixel(x, y) {
                        differing += 1;
                    }
                }
            }
            if differing > compare.max_pixels {
                return Err(format!(
                    "{differing} pixels differ (at most {} allowed)",
                    compare.max_pixels
                ));
            }
            Ok(())
        }
        Mode::Similarity => {
            const GW: usize = 64;
            const GH: usize = 48;
            let a = luminance_grid(
                ours,
                compare.crop,
                GW,
                GH,
                compare.normalize,
                compare.anchor_ground,
            );
            let b = luminance_grid(
                golden,
                compare.golden_crop.or(compare.crop),
                GW,
                GH,
                compare.normalize,
                compare.anchor_ground,
            );
            let mad = a.iter().zip(&b).map(|(p, q)| (p - q).abs()).sum::<f64>() / (GW * GH) as f64;
            if mad > compare.max_mad {
                return Err(format!(
                    "the picture differs from the golden by {mad:.2} (0 to 255) on average, at most {} allowed",
                    compare.max_mad
                ));
            }
            Ok(())
        }
    }
}

/// Writes `ours` and the golden side by side with their difference, for a failing screen scenario.
fn write_diff(path: &Path, ours: &Screen, golden: &Screen) -> Result<(), String> {
    let (w, h) = (ours.width.max(golden.width), ours.height.max(golden.height));
    let total_w = w * 3;
    let mut rgb = vec![0u8; total_w * h * 3];
    for y in 0..h {
        for x in 0..w {
            let a = (x < ours.width && y < ours.height).then(|| ours.pixel(x, y));
            let b = (x < golden.width && y < golden.height).then(|| golden.pixel(x, y));
            let put = |col: usize, p: [u8; 3], rgb: &mut Vec<u8>| {
                let i = (y * total_w + col * w + x) * 3;
                rgb[i..i + 3].copy_from_slice(&p);
            };
            put(0, a.unwrap_or([255, 0, 255]), &mut rgb);
            put(1, b.unwrap_or([255, 0, 255]), &mut rgb);
            let d = match (a, b) {
                (Some(a), Some(b)) => [
                    a[0].abs_diff(b[0]),
                    a[1].abs_diff(b[1]),
                    a[2].abs_diff(b[2]),
                ],
                _ => [255, 0, 255],
            };
            put(2, d, &mut rgb);
        }
    }
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), total_w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut w| w.write_image_data(&rgb))
        .map_err(|e| e.to_string())
}

/// Where the diff images of failing scenarios go: `RC_SCENARIO_OUT`, else the temp folder.
fn out_dir() -> PathBuf {
    std::env::var_os("RC_SCENARIO_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

/// The manifest's files with the scenario's own on top: a scenario naming the same option replaces it (a diagnostic ROM
/// instead of the machine's firmware).
fn merged_files(manifest: &Manifest, own: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut all = manifest.files.clone();
    all.extend(own.iter().map(|(k, v)| (k.clone(), v.clone())));
    all
}

fn base_run<'e>(
    env: &'e Env,
    manifest: &Manifest,
    scenario: &Scenario,
) -> Result<crate::Run<'e>, String> {
    let mut run = env.run(&manifest.machine);
    for (flag, rel) in merged_files(manifest, &scenario.files) {
        run = run
            .arg(format!("--{flag}"))
            .arg(env.file(&rel)?.to_string_lossy());
    }
    for a in manifest.args.iter().chain(&scenario.args) {
        run = run.arg(a.clone());
    }
    if scenario.frames > 0 {
        run = run.frames(scenario.frames);
    }
    for action in &scenario.actions {
        run = run.arg("--at").arg(action.clone());
    }
    Ok(run)
}

fn run_screen(env: &Env, goldens: &Path, manifest: &Manifest, s: &Scenario) -> Result<(), String> {
    let (path, _) = read_golden(
        goldens,
        s.golden.as_deref().unwrap_or_default(),
        s.golden_sha256.as_deref().unwrap_or_default(),
    )?;
    let golden = Screen::read(&path)?;
    let out = base_run(env, manifest, s)?.screen().go()?;
    let ours = out.screen()?;
    let compare = s.compare.as_ref().expect("validated");
    compare_screens(ours, &golden, compare).inspect_err(|_| {
        let diff = out_dir().join(format!("diff-{}-{}.png", manifest.machine, s.id));
        let _ = write_diff(&diff, ours, &golden);
    })
}

fn run_memory(env: &Env, goldens: &Path, manifest: &Manifest, s: &Scenario) -> Result<(), String> {
    let (_, want) = read_golden(
        goldens,
        s.golden.as_deref().unwrap_or_default(),
        s.golden_sha256.as_deref().unwrap_or_default(),
    )?;
    let (start, len) = (s.start.expect("validated"), s.len.expect("validated"));
    if want.len() != usize::from(len) {
        return Err(format!(
            "the golden holds {} bytes, `len` is {len}",
            want.len()
        ));
    }
    let out = base_run(env, manifest, s)?.dump(start, len).go()?;
    let got = out.bytes(start, len)?;
    match got.iter().zip(&want).position(|(a, b)| a != b) {
        None => Ok(()),
        Some(i) => Err(format!(
            "memory differs first at 0x{:04X}: {:02X} here, {:02X} in the golden",
            u32::from(start) + i as u32,
            got[i],
            want[i]
        )),
    }
}

fn run_audio(env: &Env, manifest: &Manifest, s: &Scenario) -> Result<(), String> {
    let want = s.audio.as_ref().expect("validated");
    let out = base_run(env, manifest, s)?.audio().go()?;
    let audio = out.audio()?;
    if let Some(limit) = want.silent_below_rms {
        let rms = audio.rms(want.from_s, want.to_s);
        if rms >= limit {
            return Err(format!(
                "expected silence below {limit}, the level is {rms:.4}"
            ));
        }
    }
    if let Some(freq) = want.tone_hz {
        let (found, amplitude) = if want.tone_tolerance_pct > 0.0 {
            let d = freq * want.tone_tolerance_pct / 100.0;
            audio.peak(freq - d, freq + d, want.from_s, want.to_s)
        } else {
            (freq, audio.tone(freq, want.from_s, want.to_s))
        };
        if amplitude < want.min_amplitude {
            return Err(format!(
                "a {freq} Hz tone (within {}%) has amplitude {amplitude:.4} at best (at {found:.2} Hz), at least {} expected",
                want.tone_tolerance_pct, want.min_amplitude
            ));
        }
    }
    Ok(())
}

fn run_timing(env: &Env, manifest: &Manifest, s: &Scenario) -> Result<(), String> {
    let out = base_run(env, manifest, s)?.go()?;
    let per_frame = out.cycles / u64::from(s.frames);
    let want = s.cycles_per_frame.expect("validated");
    if per_frame.abs_diff(want) > s.cycles_tolerance {
        return Err(format!(
            "{per_frame} cycles per frame, expected {want} (tolerance {})",
            s.cycles_tolerance
        ));
    }
    Ok(())
}

fn temp_file(tag: &str, id: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rc-scenario-{tag}-{id}-{}.state",
        std::process::id()
    ))
}

/// S5: two fresh runs of the same length save the same state, and a run that loads the first one and goes
/// on for `more_frames` ends where a fresh run of the whole length ends.
fn run_state(env: &Env, manifest: &Manifest, s: &Scenario) -> Result<(), String> {
    let (a, b, c, d) = (
        temp_file("a", &s.id),
        temp_file("b", &s.id),
        temp_file("c", &s.id),
        temp_file("d", &s.id),
    );
    let result = (|| {
        base_run(env, manifest, s)?
            .arg("--dump-state")
            .arg(a.to_string_lossy())
            .go()?;
        base_run(env, manifest, s)?
            .arg("--dump-state")
            .arg(b.to_string_lossy())
            .go()?;
        let (sa, sb) = (read(&a)?, read(&b)?);
        if sa != sb {
            return Err("two runs of the same length saved different states".to_string());
        }
        if s.more_frames == 0 {
            return Ok(());
        }
        let mut resumed = env.run(&manifest.machine);
        for (flag, rel) in merged_files(manifest, &s.files) {
            resumed = resumed
                .arg(format!("--{flag}"))
                .arg(env.file(&rel)?.to_string_lossy());
        }
        for arg in manifest.args.iter().chain(&s.args) {
            resumed = resumed.arg(arg.clone());
        }
        resumed
            .arg("--load-state")
            .arg(a.to_string_lossy())
            .frames(s.more_frames)
            .arg("--dump-state")
            .arg(c.to_string_lossy())
            .go()?;
        let mut whole = s.clone();
        whole.frames = s.frames + s.more_frames;
        base_run(env, manifest, &whole)?
            .arg("--dump-state")
            .arg(d.to_string_lossy())
            .go()?;
        if read(&c)? != read(&d)? {
            return Err(
                "a run that loads the saved state and continues ends in a different state than a run \
                 of the whole length"
                    .to_string(),
            );
        }
        Ok(())
    })();
    for f in [&a, &b, &c, &d] {
        let _ = std::fs::remove_file(f);
    }
    result
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("no state file {}: {e}", path.display()))
}

/// Runs one scenario.
pub fn run_scenario(
    env: &Env,
    goldens: &Path,
    manifest: &Manifest,
    s: &Scenario,
) -> Result<(), String> {
    if matches!(s.kind, Kind::Screen | Kind::Memory) && s.golden.is_none() {
        return Err(
            "not run: reference golden not captured; capture and pin it before comparison"
                .to_string(),
        );
    }
    if matches!(s.kind, Kind::Audio) && s.audio.is_none() {
        return Err(
            "not run: reference audio expectations not captured; measure them before comparison"
                .to_string(),
        );
    }
    match s.kind {
        Kind::Screen => run_screen(env, goldens, manifest, s),
        Kind::Memory => run_memory(env, goldens, manifest, s),
        Kind::Audio => run_audio(env, manifest, s),
        Kind::Timing => run_timing(env, manifest, s),
        Kind::State => run_state(env, manifest, s),
    }
}

/// Sorts a scenario's result with its declared status into the report.
pub fn record(report: &mut ScenarioReport, s: &Scenario, result: Result<(), String>) {
    if let Err(e) = &result
        && !e.contains(FIRMWARE_WRONG_HASH)
        && let Some(line) = e.lines().find(|l| l.contains(FIRMWARE_UNAVAILABLE))
    {
        report.skipped.push((
            s.id.clone(),
            format!(
                "firmware not available (no cache, RomM or network): {}",
                line.trim()
            ),
        ));
        return;
    }
    match (result, s.status) {
        (Ok(()), Status::Pass) => report.passed.push(s.id.clone()),
        (Err(e), Status::Pass) => report.failed.push((s.id.clone(), e)),
        (Err(e), Status::KnownGap) => {
            let gap = s.gap.as_ref().expect("validated");
            report.gaps.push((
                s.id.clone(),
                format!("{} (evidence: {})", gap.reason, gap.evidence),
                e,
            ));
        }
        (Ok(()), Status::KnownGap) => report.closed.push(s.id.clone()),
    }
}

/// Runs every scenario of `manifest`; the caller asserts [`ScenarioReport::ok`].
pub fn run_manifest(env: &Env, goldens: &Path, manifest: &Manifest) -> ScenarioReport {
    let mut report = ScenarioReport::default();
    for s in &manifest.scenarios {
        let result = run_scenario(env, goldens, manifest, s);
        record(&mut report, s, result);
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(width: usize, height: usize, f: impl Fn(usize, usize) -> [u8; 3]) -> Screen {
        let mut rgb = Vec::with_capacity(width * height * 3);
        for y in 0..height {
            for x in 0..width {
                rgb.extend_from_slice(&f(x, y));
            }
        }
        Screen::from_rgb(width, height, rgb)
    }

    fn compare(mode: Mode, max_pixels: usize, max_mad: f64) -> Compare {
        Compare {
            mode,
            max_pixels,
            max_mad,
            crop: None,
            golden_crop: None,
            normalize: false,
            anchor_ground: false,
            reason: "test".to_string(),
        }
    }

    #[test]
    fn anchor_ground_ignores_the_paper_grey_even_beside_a_brighter_white() {
        // Black text and a pure white bar on a 0xCD ground against the same on a 0xC0 ground.
        let draw = |ground: u8, text: bool| {
            screen(64, 48, |x, y| {
                if y < 4 {
                    [255, 255, 255]
                } else if text && x < 8 {
                    [0, 0, 0]
                } else {
                    [ground; 3]
                }
            })
        };
        let mut c = compare(Mode::Similarity, 0, 2.0);
        c.normalize = true;
        assert!(compare_screens(&draw(0xCD, true), &draw(0xC0, true), &c).is_err());
        c.normalize = false;
        c.anchor_ground = true;
        assert!(compare_screens(&draw(0xCD, true), &draw(0xC0, true), &c).is_ok());
        assert!(compare_screens(&draw(0xCD, false), &draw(0xC0, true), &c).is_err());
    }

    #[test]
    fn normalize_ignores_a_palette_difference_but_not_a_missing_shape() {
        // Black text on a 0xCD ground against the same text on a 0xC0 ground: only the palette differs.
        let draw = |ground: u8, text: bool| {
            screen(64, 48, |x, _| {
                if text && x < 8 {
                    [0, 0, 0]
                } else {
                    [ground; 3]
                }
            })
        };
        let mut c = compare(Mode::Similarity, 0, 2.0);
        assert!(compare_screens(&draw(0xCD, true), &draw(0xC0, true), &c).is_err());
        c.normalize = true;
        assert!(compare_screens(&draw(0xCD, true), &draw(0xC0, true), &c).is_ok());
        assert!(compare_screens(&draw(0xCD, false), &draw(0xC0, true), &c).is_err());
    }

    const MINIMAL: &str = r#"
machine = "zx81"
[[scenario]]
id = "S6"
kind = "timing"
frames = 10
cycles_per_frame = 65000
"#;

    #[test]
    fn a_minimal_manifest_parses_and_defaults_to_a_pass() {
        let m = Manifest::parse(MINIMAL).unwrap();
        assert_eq!(m.machine, "zx81");
        assert_eq!(m.scenarios[0].status, Status::Pass);
        assert_eq!(m.scenarios[0].cycles_tolerance, 0);
    }

    #[test]
    fn an_uncaptured_golden_is_an_explicit_gap_not_a_fabricated_hash_or_pass() {
        for kind in ["screen", "memory", "audio"] {
            let source = format!(
                r#"
machine = "vic20"
[[scenario]]
id = "S1"
kind = "{kind}"
frames = 180
start = 0
len = 1
status = "known-gap"
gap = {{ reason = "Reference capture unavailable", evidence = "docs/vic20-gaps.md" }}
"#
            );
            let mut manifest = Manifest::parse(&source).unwrap();
            let env = Env {
                bin: PathBuf::from("must-not-run"),
                roms: PathBuf::from("missing-roms"),
            };
            let report = run_manifest(&env, Path::new("missing-goldens"), &manifest);
            assert!(report.passed.is_empty());
            assert!(report.failed.is_empty());
            assert_eq!(report.gaps.len(), 1);
            assert!(report.gaps[0].2.contains("not run: reference"));
            manifest.scenarios[0].status = Status::Pass;
            manifest.scenarios[0].gap = None;
            assert!(manifest.validate().is_err());
            assert!(Manifest::parse(&source.replace("Reference capture unavailable", "")).is_err());
            assert!(Manifest::parse(&source.replace("docs/vic20-gaps.md", "")).is_err());
        }
    }

    #[test]
    fn the_rules_of_a_manifest_are_enforced_before_anything_runs() {
        let bad = |extra: &str| Manifest::parse(&format!("machine = \"x\"\n{extra}")).unwrap_err();
        // A golden without its pin.
        assert!(
            bad("[[scenario]]\nid=\"S1\"\nkind=\"screen\"\ngolden=\"a.png\"\ncompare={mode=\"exact\"}\n")
                .contains("pinned")
        );
        // A tolerance without a reason.
        assert!(
            bad("[[scenario]]\nid=\"S1\"\nkind=\"screen\"\ngolden=\"a.png\"\ngolden_sha256=\"00\"\ncompare={mode=\"similarity\",max_mad=5.0}\n")
                .contains("reason")
        );
        // A known gap without evidence.
        assert!(
            bad("[[scenario]]\nid=\"S6\"\nkind=\"timing\"\nframes=1\ncycles_per_frame=1\nstatus=\"known-gap\"\n")
                .contains("gap")
        );
        // The same id twice.
        let twice = format!(
            "{MINIMAL}\n[[scenario]]\nid = \"S6\"\nkind = \"timing\"\nframes = 1\ncycles_per_frame = 1\n"
        );
        assert!(Manifest::parse(&twice).unwrap_err().contains("twice"));
        // A key that does not exist is a typo, not a setting.
        assert!(Manifest::parse(&format!("{MINIMAL}\nbogus = 1\n")).is_err());
    }

    #[test]
    fn a_golden_is_pinned_by_its_hash() {
        let dir = std::env::temp_dir().join(format!("rc-scenario-pin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("g.bin"), b"abc").unwrap();
        // SHA-256 of "abc".
        let pin = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(sha256_hex(b"abc"), pin);
        assert!(read_golden(&dir, "g.bin", pin).is_ok());
        assert!(
            read_golden(&dir, "g.bin", &"0".repeat(64))
                .unwrap_err()
                .contains("pins")
        );
        assert!(
            read_golden(&dir, "missing.bin", pin)
                .unwrap_err()
                .contains("missing golden")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn exact_comparison_counts_differing_pixels_and_sizes() {
        let a = screen(8, 8, |x, _| [x as u8 * 10, 0, 0]);
        let same = screen(8, 8, |x, _| [x as u8 * 10, 0, 0]);
        let mut one = screen(8, 8, |x, _| [x as u8 * 10, 0, 0]);
        one.set_pixel(3, 3, [255, 255, 255]);
        let exact = compare(Mode::Exact, 0, 0.0);
        assert!(compare_screens(&a, &same, &exact).is_ok());
        assert!(
            compare_screens(&a, &one, &exact)
                .unwrap_err()
                .contains("1 pixels differ")
        );
        assert!(compare_screens(&a, &one, &compare(Mode::Exact, 1, 0.0)).is_ok());
        let wider = screen(9, 8, |_, _| [0, 0, 0]);
        assert!(
            compare_screens(&a, &wider, &exact)
                .unwrap_err()
                .contains("8x8")
        );
    }

    #[test]
    fn similarity_ignores_scale_and_border_but_not_content() {
        // A picture with a bright block on the left, drawn at two sizes with different borders.
        let small = screen(64, 48, |x, y| {
            if (8..24).contains(&x) && (8..40).contains(&y) {
                [255, 255, 255]
            } else {
                [0, 0, 0]
            }
        });
        let big = screen(256, 192, |x, y| {
            if (32..96).contains(&x) && (32..160).contains(&y) {
                [255, 255, 255]
            } else {
                [0, 0, 0]
            }
        });
        let moved = screen(64, 48, |x, y| {
            if (40..56).contains(&x) && (8..40).contains(&y) {
                [255, 255, 255]
            } else {
                [0, 0, 0]
            }
        });
        let sim = compare(Mode::Similarity, 0, 4.0);
        assert!(compare_screens(&small, &big, &sim).is_ok());
        assert!(
            compare_screens(&small, &moved, &sim)
                .unwrap_err()
                .contains("differs from the golden")
        );
        // Cropping compares only the chosen part: the two halves of `moved` and `small` agree on the right edge.
        let mut cropped = sim.clone();
        cropped.crop = Some([0.9, 0.0, 1.0, 1.0]);
        assert!(compare_screens(&small, &moved, &cropped).is_ok());
    }

    #[test]
    fn a_result_is_sorted_by_its_declared_status() {
        let mut m = Manifest::parse(MINIMAL).unwrap();
        let mut gap = m.scenarios[0].clone();
        gap.status = Status::KnownGap;
        gap.gap = Some(Gap {
            reason: "no reference".to_string(),
            evidence: "notes/S6.md".to_string(),
        });
        let mut report = ScenarioReport::default();
        record(&mut report, &m.scenarios[0], Ok(()));
        record(&mut report, &m.scenarios[0], Err("off".to_string()));
        record(&mut report, &gap, Err("still off".to_string()));
        assert!(!report.ok());
        report.failed.clear();
        report.gaps.clear();
        record(&mut report, &gap, Ok(()));
        assert!(!report.ok(), "a gap that passes must be unmarked");
        assert_eq!(report.passed, ["S6"]);
        m.scenarios.clear();
    }

    #[test]
    fn firmware_that_cannot_be_found_skips_the_scenario() {
        let m = Manifest::parse(MINIMAL).unwrap();
        let err = "retroclawd atari-st --headless --firmware auto failed: firmware: no RomM server (...)\n\
                   --firmware auto: no firmware with a matching SHA-1 for: rom (wanted: EmuTOS 1.4 192K)\n\
                   see `retroclawd romm match atari-st`";
        let mut report = ScenarioReport::default();
        record(&mut report, &m.scenarios[0], Err(err.to_string()));
        assert!(report.ok(), "{}", report.summary());
        assert!(report.failed.is_empty());
        assert_eq!(report.skipped.len(), 1);
        let summary = report.summary();
        assert!(summary.contains("1 skipped"), "{summary}");
        assert!(
            summary.contains("SKIP   S6: firmware not available"),
            "{summary}"
        );
        assert!(
            summary.contains("no firmware with a matching SHA-1 for: rom"),
            "the host's own line is kept: {summary}"
        );
    }

    #[test]
    fn firmware_with_the_wrong_hash_fails_the_scenario() {
        let m = Manifest::parse(MINIMAL).unwrap();
        let err = "retroclawd atari-st --headless --firmware auto failed: 
                   --firmware auto: no firmware with a matching SHA-1 for: rom (wanted: EmuTOS 1.4 192K)                    [EmuTOS 1.4 192K: https://example/e.zip has SHA-1 0000 but firmware.toml pins 904e]";
        let mut report = ScenarioReport::default();
        record(&mut report, &m.scenarios[0], Err(err.to_string()));
        assert!(report.skipped.is_empty(), "{}", report.summary());
        assert_eq!(report.failed.len(), 1);
        assert!(!report.ok());
    }
}
