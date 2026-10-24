//! A small harness for black-box emulator suites.
//!
//! A suite is a list of [`Case`]s. Each case starts the real `retroclawd` binary headless on a machine, then
//! checks memory, registers or the picture against **documented hardware facts** -- a value from a datasheet, a
//! ROM listing or the machine's manual, named in the case -- never against what the emulator printed before. A
//! case belongs to an [`Area`], so a failure says what is wrong (CPU, video, timers, ...), not just which test.
//!
//! The suites need a built `retroclawd`, its machine libraries and the firmware, so they run only when the
//! environment says where those are and skip otherwise (public CI stays green without any ROM):
//!
//! ```text
//! RETROCLAWD_BIN=.../retroclawd RETROCLAWD_MACHINE_PLUGIN_DIRS=.../plugins RC_ROMS=.../machines \
//!   cargo test -p rc-suite -- --nocapture
//! ```
//!
//! `RC_ROMS` is a folder holding `sinclair/ amstrad/ commodore/` (the family checkouts), each with its `roms/`.
//!
//! A feature the emulator does not have yet is a **gap**: the case is written against the hardware fact and marked
//! [`Case::gap`]. A gap is expected to fail; when it starts passing the run fails and says to drop the marker, so
//! the list of gaps can only shrink on purpose.

pub mod scenario;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// What a case exercises, shown with every failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Area {
    Cpu,
    Memory,
    Rom,
    Video,
    Sound,
    Timers,
    Io,
    Media,
}

/// Where the emulator and the firmware are.
pub struct Env {
    pub bin: PathBuf,
    pub roms: PathBuf,
}

impl Env {
    /// `None` when `RETROCLAWD_BIN` or `RC_ROMS` is not set: the suite then skips.
    pub fn discover() -> Option<Env> {
        Some(Env {
            bin: PathBuf::from(std::env::var_os("RETROCLAWD_BIN")?),
            roms: PathBuf::from(std::env::var_os("RC_ROMS")?),
        })
    }

    /// A file below `RC_ROMS`, e.g. `sinclair/roms/zx81/zx81.rom`; an error naming it when it is missing.
    pub fn file(&self, rel: &str) -> Result<PathBuf, String> {
        let p = self.roms.join(rel);
        if p.is_file() {
            Ok(p)
        } else {
            Err(format!("missing firmware file {}", p.display()))
        }
    }

    /// Starts a headless run of `machine`.
    pub fn run(&self, machine: &str) -> Run<'_> {
        Run {
            env: self,
            args: vec![machine.to_string(), "--headless".to_string()],
            dumps: Vec::new(),
            screen: false,
            audio: false,
        }
    }
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Assembles Z80 `source` at `org` (with `.org` and `.cpu z80` supplied) and returns the bytes.
pub fn z80_image(org: u16, source: &str) -> Result<Vec<u8>, String> {
    let opts = rcasm::Options {
        cpu: Some(rcasm::Cpu::Z80),
        ..rcasm::Options::default()
    };
    let asm = rcasm::assemble(
        "case.asm",
        &format!(
            ".org ${org:04X}
{source}"
        ),
        &opts,
        &rcasm::DiskFiles,
    )
    .map_err(|e| format!("{e:?}"))?;
    let (start, image) = rcasm::flat_image(&asm, 0)?;
    if start != org {
        return Err(format!(
            "the program starts at 0x{start:04X}, not 0x{org:04X}"
        ));
    }
    Ok(image)
}

/// Assembles Z80 `source` at `org` (with `.org` and `.cpu z80` supplied) into a temporary file for `--program`.
pub fn z80_program(tag: &str, org: u16, source: &str) -> Result<PathBuf, String> {
    let opts = rcasm::Options {
        cpu: Some(rcasm::Cpu::Z80),
        ..rcasm::Options::default()
    };
    let asm = rcasm::assemble(
        "case.asm",
        &format!(
            ".org ${org:04X}
{source}"
        ),
        &opts,
        &rcasm::DiskFiles,
    )
    .map_err(|e| format!("{e:?}"))?;
    let (start, image) = rcasm::flat_image(&asm, 0)?;
    if start != org {
        return Err(format!(
            "the program starts at 0x{start:04X}, not 0x{org:04X}"
        ));
    }
    let path = std::env::temp_dir().join(format!(
        "rc-suite-{tag}-{}-{}.bin",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, image).map_err(|e| e.to_string())?;
    Ok(path)
}

/// One headless emulator invocation, built up and then [`Run::go`].
pub struct Run<'e> {
    env: &'e Env,
    args: Vec<String>,
    dumps: Vec<(u32, u32)>,
    screen: bool,
    audio: bool,
}

impl Run<'_> {
    pub fn arg(mut self, a: impl Into<String>) -> Self {
        self.args.push(a.into());
        self
    }

    pub fn rom(self, path: &Path) -> Self {
        self.arg("--rom").arg(path.to_string_lossy())
    }

    /// Run whole frames, like the window (frame interrupt each, no stop at HALT).
    pub fn frames(self, n: u32) -> Self {
        self.arg("--frames").arg(n.to_string())
    }

    /// Read `len` bytes from `start` after the run.
    pub fn dump(self, start: u16, len: u16) -> Self {
        self.dump32(u32::from(start), u32::from(len))
    }

    /// `dump` for a machine with a wider address space (the 68000's work RAM is at `$FF0000`).
    pub fn dump32(mut self, start: u32, len: u32) -> Self {
        self.dumps.push((start, len));
        self
    }

    /// Also capture the picture.
    pub fn screen(mut self) -> Self {
        self.screen = true;
        self
    }

    /// Also capture everything the machine played (`--dump-audio`, needs `--frames`).
    pub fn audio(mut self) -> Self {
        self.audio = true;
        self
    }

    /// Runs the emulator once per dump range (the CLI dumps one range at a time). A run is deterministic, so
    /// the ranges all describe the same moment.
    pub fn go(self) -> Result<Outcome, String> {
        let dumps = if self.dumps.is_empty() {
            vec![(0, 0)]
        } else {
            self.dumps.clone()
        };
        let mut out = Outcome::default();
        for (i, (start, len)) in dumps.iter().enumerate() {
            let mut args = self.args.clone();
            if *len > 0 {
                args.push("--dump-memory".to_string());
                args.push(format!("0x{start:04X}:{len}"));
            }
            let shot = std::env::temp_dir().join(format!(
                "rc-suite-{}-{}.png",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            let want_audio = self.audio && i == 0;
            let wav = std::env::temp_dir().join(format!(
                "rc-suite-{}-{}.wav",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            if want_audio {
                args.push("--dump-audio".to_string());
                args.push(wav.to_string_lossy().into_owned());
            }
            let want_screen = self.screen && i == 0;
            if want_screen {
                args.push("--dump-screen".to_string());
                args.push(shot.to_string_lossy().into_owned());
            }
            let cmd = Command::new(&self.env.bin)
                .args(&args)
                .output()
                .map_err(|e| format!("cannot start {}: {e}", self.env.bin.display()))?;
            let stdout = String::from_utf8_lossy(&cmd.stdout).into_owned();
            if !cmd.status.success() {
                return Err(format!(
                    "retroclawd {} failed: {}",
                    args.join(" "),
                    String::from_utf8_lossy(&cmd.stderr).trim()
                ));
            }
            parse(&stdout, &mut out)?;
            if want_audio {
                out.audio = Some(Audio::read(&wav)?);
                let _ = std::fs::remove_file(&wav);
            }
            if want_screen {
                out.screen = Some(Screen::read(&shot)?);
                let _ = std::fs::remove_file(&shot);
            }
        }
        Ok(out)
    }
}

/// What a run left behind.
#[derive(Default)]
pub struct Outcome {
    pub registers: HashMap<String, u16>,
    pub cycles: u64,
    memory: HashMap<u32, u8>,
    pub screen: Option<Screen>,
    pub audio: Option<Audio>,
}

impl Outcome {
    pub fn byte(&self, addr: u16) -> Result<u8, String> {
        self.byte32(u32::from(addr))
    }

    pub fn byte32(&self, addr: u32) -> Result<u8, String> {
        self.memory
            .get(&addr)
            .copied()
            .ok_or_else(|| format!("0x{addr:04X} was not dumped"))
    }

    pub fn word(&self, addr: u16) -> Result<u16, String> {
        Ok(u16::from(self.byte(addr)?) | (u16::from(self.byte(addr.wrapping_add(1))?) << 8))
    }

    pub fn bytes(&self, addr: u16, len: u16) -> Result<Vec<u8>, String> {
        (0..len).map(|i| self.byte(addr.wrapping_add(i))).collect()
    }

    pub fn bytes32(&self, addr: u32, len: u32) -> Result<Vec<u8>, String> {
        (0..len).map(|i| self.byte32(addr.wrapping_add(i))).collect()
    }

    pub fn reg(&self, name: &str) -> Result<u16, String> {
        self.registers
            .get(name)
            .copied()
            .ok_or_else(|| format!("no register {name} in the output"))
    }

    pub fn screen(&self) -> Result<&Screen, String> {
        self.screen
            .as_ref()
            .ok_or_else(|| "no screen was captured".to_string())
    }

    pub fn audio(&self) -> Result<&Audio, String> {
        self.audio
            .as_ref()
            .ok_or_else(|| "no audio was captured".to_string())
    }
}

/// What a run played: mono samples in -1..1.
pub struct Audio {
    pub rate: u32,
    /// Interleaved when the file has more than one channel; `channel` picks one out.
    pub samples: Vec<f32>,
    pub channels: usize,
}

impl Audio {
    /// Reads the 16-bit PCM `.wav` that `--dump-audio` writes (mono or interleaved stereo).
    pub fn read(path: &Path) -> Result<Audio, String> {
        let bytes =
            std::fs::read(path).map_err(|e| format!("no audio file {}: {e}", path.display()))?;
        if bytes.len() < 44 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
            return Err("not a .wav file".to_string());
        }
        let rate = u32::from_le_bytes(bytes[24..28].try_into().expect("4 bytes"));
        let channels = usize::from(u16::from_le_bytes(
            bytes[22..24].try_into().expect("2 bytes"),
        ))
        .max(1);
        let samples = bytes[44..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| f32::from(i16::from_le_bytes(*b)) / 32768.0)
            .collect();
        Ok(Audio {
            rate,
            samples,
            channels,
        })
    }

    /// One channel of the file as a mono signal at the same rate.
    pub fn channel(&self, n: usize) -> Result<Audio, String> {
        if n >= self.channels {
            return Err(format!(
                "the audio has {} channel(s), not channel {n}",
                self.channels
            ));
        }
        Ok(Audio {
            rate: self.rate,
            samples: self
                .samples
                .iter()
                .skip(n)
                .step_by(self.channels)
                .copied()
                .collect(),
            channels: 1,
        })
    }

    /// The samples from `from_s` to `to_s` seconds.
    pub fn window(&self, from_s: f64, to_s: f64) -> &[f32] {
        let a = ((from_s * f64::from(self.rate)) as usize).min(self.samples.len());
        let b = ((to_s * f64::from(self.rate)) as usize).clamp(a, self.samples.len());
        &self.samples[a..b]
    }

    /// Root-mean-square level of the samples from `from_s` to `to_s` with their mean (the DC offset) taken out: a PSG whose
    /// channels are all disabled holds its output at a constant level, as the real chip does, which is silence to the ear.
    pub fn rms(&self, from_s: f64, to_s: f64) -> f64 {
        let w = self.window(from_s, to_s);
        if w.is_empty() {
            return 0.0;
        }
        let mean = w.iter().map(|&x| f64::from(x)).sum::<f64>() / w.len() as f64;
        (w.iter()
            .map(|&x| (f64::from(x) - mean).powi(2))
            .sum::<f64>()
            / w.len() as f64)
            .sqrt()
    }

    /// The amplitude of a sine of `freq` Hz in the samples from `from_s` to `to_s` (a single-bin Goertzel/DFT): a square
    /// wave of that frequency and peak `p` reads about `4p/pi`, a tone at another frequency near zero.
    pub fn tone(&self, freq: f64, from_s: f64, to_s: f64) -> f64 {
        let w = self.window(from_s, to_s);
        if w.is_empty() {
            return 0.0;
        }
        let (mut re, mut im) = (0.0, 0.0);
        for (n, &x) in w.iter().enumerate() {
            let phase = 2.0 * std::f64::consts::PI * freq * n as f64 / f64::from(self.rate);
            re += f64::from(x) * phase.cos();
            im += f64::from(x) * phase.sin();
        }
        2.0 * (re * re + im * im).sqrt() / w.len() as f64
    }

    /// The frequency of the strongest component between `lo_hz` and `hi_hz` over `from_s` to `to_s`, from an FFT of the
    /// first power-of-two run of samples (about 1.5 Hz resolution at 44.1 kHz); refine it with `peak`.
    pub fn dominant(&self, lo_hz: f64, hi_hz: f64, from_s: f64, to_s: f64) -> f64 {
        let w = self.window(from_s, to_s);
        let mut n = 1usize;
        while n * 2 <= w.len().min(1 << 15) {
            n *= 2;
        }
        if n < 8 {
            return lo_hz;
        }
        let mean = w[..n].iter().map(|&x| f64::from(x)).sum::<f64>() / n as f64;
        let mut re: Vec<f64> = w[..n].iter().map(|&x| f64::from(x) - mean).collect();
        let mut im = vec![0.0; n];
        let bits = n.trailing_zeros();
        for i in 0..n {
            let j = i.reverse_bits() >> (usize::BITS - bits);
            if j > i {
                re.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let ang = -2.0 * std::f64::consts::PI / len as f64;
            for start in (0..n).step_by(len) {
                for k in 0..len / 2 {
                    let (wr, wi) = ((ang * k as f64).cos(), (ang * k as f64).sin());
                    let (a, b) = (start + k, start + k + len / 2);
                    let (tr, ti) = (re[b] * wr - im[b] * wi, re[b] * wi + im[b] * wr);
                    (re[b], im[b]) = (re[a] - tr, im[a] - ti);
                    (re[a], im[a]) = (re[a] + tr, im[a] + ti);
                }
            }
            len *= 2;
        }
        let bin_hz = f64::from(self.rate) / n as f64;
        let (mut best, mut best_f) = (0.0, lo_hz);
        for k in 1..n / 2 {
            let f = k as f64 * bin_hz;
            if f >= lo_hz && f <= hi_hz {
                let m = re[k] * re[k] + im[k] * im[k];
                if m > best {
                    (best, best_f) = (m, f);
                }
            }
        }
        best_f
    }

    /// The strongest sine in `lo_hz..=hi_hz` over `from_s` to `to_s`: `(frequency, amplitude)`. Scans in steps of a quarter of
    /// the window's own resolution (1 / length), so two emulators whose clocks differ by a fraction of a percent can both be
    /// measured without knowing the exact pitch.
    pub fn peak(&self, lo_hz: f64, hi_hz: f64, from_s: f64, to_s: f64) -> (f64, f64) {
        let len_s = (to_s - from_s).max(0.001);
        let step = 0.25 / len_s;
        let mut best = (lo_hz, 0.0);
        let mut f = lo_hz;
        while f <= hi_hz {
            let a = self.tone(f, from_s, to_s);
            if a > best.1 {
                best = (f, a);
            }
            f += step;
        }
        best
    }
}

/// Reads `retroclawd --headless`'s text output: the cycle count, `NAME = 0xVALUE` registers and `0xADDR: ..` lines.
fn parse(stdout: &str, out: &mut Outcome) -> Result<(), String> {
    for line in stdout.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("ran ") {
            // "ran 100 frames, 6500439 cycles, 0.285s elapsed"
            if let Some(c) = rest
                .split(", ")
                .nth(1)
                .and_then(|s| s.strip_suffix(" cycles"))
            {
                out.cycles = c.parse().map_err(|_| format!("bad cycle count in {t:?}"))?;
            }
        } else if let Some((_, rest)) = t.split_once(" instructions, ")
            && let Some(c) = rest
                .split(", ")
                .next()
                .and_then(|s| s.strip_suffix(" cycles"))
        {
            // a run to HALT: "3583 instructions, 14356 cycles, 0.021s elapsed (~696687 Hz effective)"
            out.cycles = c.parse().map_err(|_| format!("bad cycle count in {t:?}"))?;
        } else if let Some((name, value)) = t.split_once(" = 0x") {
            if let Ok(v) = u16::from_str_radix(value.trim(), 16) {
                out.registers.insert(name.trim().to_string(), v);
            }
        } else if let Some(rest) = t.strip_prefix("0x")
            && let Some((addr, bytes)) = rest.split_once(": ")
            && let Ok(mut a) = u32::from_str_radix(addr, 16)
        {
            for b in bytes.split_whitespace() {
                let v =
                    u8::from_str_radix(b, 16).map_err(|_| format!("bad byte {b:?} in {t:?}"))?;
                out.memory.insert(a, v);
                a = a.wrapping_add(1);
            }
        }
    }
    Ok(())
}

/// A captured picture, 8 bits per channel RGB.
pub struct Screen {
    pub width: usize,
    pub height: usize,
    rgb: Vec<u8>,
}

impl Screen {
    /// Reads a PNG written by `--dump-screen` or `--at N:screenshot=FILE`.
    pub fn read(path: &Path) -> Result<Screen, String> {
        let file = std::fs::File::open(path)
            .map_err(|e| format!("no screen file {}: {e}", path.display()))?;
        let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
        // Palette (indexed) and low-bit-depth pictures become 8-bit RGB(A): Hatari writes indexed PNGs.
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
        let mut buf = vec![0; reader.output_buffer_size().ok_or("screen too large")?];
        let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
        let channels = info.color_type.samples();
        let mut rgb = Vec::with_capacity(info.width as usize * info.height as usize * 3);
        for px in buf[..info.buffer_size()].chunks(channels) {
            let (r, g, b) = if channels >= 3 {
                (px[0], px[1], px[2])
            } else {
                (px[0], px[0], px[0])
            };
            rgb.extend_from_slice(&[r, g, b]);
        }
        Ok(Screen {
            width: info.width as usize,
            height: info.height as usize,
            rgb,
        })
    }

    /// A picture from 8-bit RGB rows (`width * height * 3` bytes).
    pub fn from_rgb(width: usize, height: usize, rgb: Vec<u8>) -> Screen {
        assert_eq!(
            rgb.len(),
            width * height * 3,
            "RGB data does not fit the size"
        );
        Screen { width, height, rgb }
    }

    pub fn set_pixel(&mut self, x: usize, y: usize, p: [u8; 3]) {
        let i = (y * self.width + x) * 3;
        self.rgb[i..i + 3].copy_from_slice(&p);
    }

    pub fn pixel(&self, x: usize, y: usize) -> [u8; 3] {
        let i = (y * self.width + x) * 3;
        [self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]]
    }

    /// Dark pixel: a monochrome machine's ink.
    pub fn is_ink(&self, x: usize, y: usize) -> bool {
        let p = self.pixel(x, y);
        u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]) < 3 * 128
    }

    /// The 8x8 cell at pixel origin (`x`, `y`) as 8 rows of bits, bit 7 the left pixel, 1 = ink.
    pub fn cell(&self, x: usize, y: usize) -> [u8; 8] {
        let mut rows = [0u8; 8];
        for (r, row) in rows.iter_mut().enumerate() {
            for c in 0..8 {
                if self.is_ink(x + c, y + r) {
                    *row |= 0x80 >> c;
                }
            }
        }
        rows
    }

    /// How many pixels are ink.
    pub fn ink_count(&self) -> usize {
        (0..self.height)
            .map(|y| (0..self.width).filter(|&x| self.is_ink(x, y)).count())
            .sum()
    }
}

/// One checked fact.
pub struct Case {
    pub area: Area,
    pub name: &'static str,
    /// The hardware fact and where it comes from.
    pub source: &'static str,
    /// Set while the emulator lacks the feature: why, and which roadmap item closes it.
    pub gap: Option<&'static str>,
    pub run: fn(&Env) -> Result<(), String>,
}

impl Case {
    pub const fn new(
        area: Area,
        name: &'static str,
        source: &'static str,
        run: fn(&Env) -> Result<(), String>,
    ) -> Case {
        Case {
            area,
            name,
            source,
            gap: None,
            run,
        }
    }

    /// The same case, expected to fail until the feature exists.
    pub const fn gap(mut self, why: &'static str) -> Case {
        self.gap = Some(why);
        self
    }
}

/// How a suite ended.
#[derive(Default)]
pub struct Report {
    pub passed: Vec<&'static str>,
    pub failed: Vec<(Area, &'static str, String)>,
    /// Gaps that still fail, as expected.
    pub gaps: Vec<(&'static str, &'static str, String)>,
    /// Gaps that now pass: the marker must go.
    pub closed: Vec<&'static str>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.failed.is_empty() && self.closed.is_empty()
    }

    pub fn summary(&self) -> String {
        let mut s = format!(
            "{} passed, {} failed, {} known gaps, {} gaps now closed\n",
            self.passed.len(),
            self.failed.len(),
            self.gaps.len(),
            self.closed.len()
        );
        for (area, name, why) in &self.failed {
            s += &format!("  FAIL   [{area:?}] {name}: {why}\n");
        }
        for name in &self.closed {
            s += &format!("  CLOSED {name}: it passes now, remove its .gap(..) marker\n");
        }
        for (name, why, got) in &self.gaps {
            s += &format!("  gap    {name}: {why}\n           fails now with: {got}\n");
        }
        s
    }
}

/// Runs every case; the caller asserts [`Report::ok`].
pub fn run_cases(env: &Env, cases: &[Case]) -> Report {
    let mut report = Report::default();
    for case in cases {
        match ((case.run)(env), case.gap) {
            (Ok(()), None) => report.passed.push(case.name),
            (Err(e), None) => report.failed.push((case.area, case.name, e)),
            (Err(e), Some(why)) => report.gaps.push((case.name, why, e)),
            (Ok(()), Some(_)) => report.closed.push(case.name),
        }
    }
    report
}

/// `Ok` when `got == want`, else a message naming what was compared.
pub fn expect_eq<T: PartialEq + std::fmt::Debug>(
    what: &str,
    got: T,
    want: T,
) -> Result<(), String> {
    if got == want {
        Ok(())
    } else {
        Err(format!("{what}: got {got:?}, hardware says {want:?}"))
    }
}

/// `Ok` when `lo <= got <= hi`.
pub fn expect_in(what: &str, got: i64, lo: i64, hi: i64) -> Result<(), String> {
    if (lo..=hi).contains(&got) {
        Ok(())
    } else {
        Err(format!("{what}: got {got}, expected {lo}..={hi}"))
    }
}

/// Assembles 6502 `source` at `org` (with `.org` and `.cpu 6502` supplied) and returns the bytes.
pub fn mos6502_image(org: u16, source: &str) -> Result<Vec<u8>, String> {
    let opts = rcasm::Options {
        cpu: Some(rcasm::Cpu::Mos6502),
        ..rcasm::Options::default()
    };
    let asm = rcasm::assemble(
        "case.asm",
        &format!(".org ${org:04X}\n{source}"),
        &opts,
        &rcasm::DiskFiles,
    )
    .map_err(|e| format!("{e:?}"))?;
    let (start, image) = rcasm::flat_image(&asm, 0)?;
    if start != org {
        return Err(format!(
            "the program starts at 0x{start:04X}, not 0x{org:04X}"
        ));
    }
    Ok(image)
}

/// Assembles 6502 `source` at `org` into a temporary file for `--program` (a raw image, loaded at `--org`).
pub fn mos6502_program(tag: &str, org: u16, source: &str) -> Result<PathBuf, String> {
    let image = mos6502_image(org, source)?;
    let path = std::env::temp_dir().join(format!(
        "rc-suite-{tag}-{}-{}.bin",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, image).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Where a C128 test program's 6502 code starts, `C128_CODE_OFFSET` bytes after the Z80 stub's `org`.
pub const C128_CODE_OFFSET: u16 = 0x100;

/// A C128 program for `--program`: a C128 powers up on its Z80, which this stub (at `org`) uses to set the map the
/// 8502 will see and then hand the machine over: configuration register = 0x3E (all RAM, bank 0, I/O visible), the
/// 8502 reset vector at 0xFFFC pointing at the 6502 `source` (assembled at `org + C128_CODE_OFFSET`), then the
/// processor-select bit of the mode configuration register set (a Z80 reaches the I/O chips with `IN`/`OUT`, the port's
/// high byte choosing the chip, so that is a port access, not a memory write). The 8502 starts from that vector (MMU: "the Z80
/// releases the machine; the 8502 starts as from reset"), so the 6502 code runs on a machine nothing has initialised.
pub fn c128_8502_program(tag: &str, org: u16, source: &str) -> Result<PathBuf, String> {
    let code_at = org + C128_CODE_OFFSET;
    let stub = z80_image(
        org,
        &format!(
            " di
 ld a,$3E
 ld ($FF00),a
 ld hl,${code_at:04X}
 ld ($FFFC),hl
 ld bc,$D505
 in a,(c)
 or 1
 out (c),a
stay: jr stay
"
        ),
    )?;
    let code = mos6502_image(code_at, source)?;
    let mut image = stub;
    image.resize(usize::from(C128_CODE_OFFSET), 0);
    image.extend_from_slice(&code);
    let path = std::env::temp_dir().join(format!(
        "rc-suite-{tag}-{}-{}.bin",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, image).map_err(|e| e.to_string())?;
    Ok(path)
}

impl Env {
    /// Starts a headless run of a C128 with its ROM set from `RC_ROMS` (`commodore/roms/c128`); the two BASIC halves
    /// are joined into one file, as the machine wants them. An error naming the missing file when a ROM is absent.
    /// A C64 run with the three ROMs from the Commodore family's `roms/c64` folder.
    pub fn c64(&self) -> Result<Run<'_>, String> {
        let dir = "commodore/roms/c64";
        let path = |name: &str| -> Result<String, String> {
            Ok(self
                .file(&format!("{dir}/{name}"))?
                .to_string_lossy()
                .into_owned())
        };
        Ok(self
            .run("c64")
            .arg("--rom-kernal")
            .arg(path("kernal-901227-03.bin")?)
            .arg("--rom-basic")
            .arg(path("basic-901226-01.bin")?)
            .arg("--rom-char")
            .arg(path("chargen-901225-01.bin")?))
    }

    pub fn c128(&self) -> Result<Run<'_>, String> {
        let dir = "commodore/roms/c128";
        let lo = std::fs::read(self.file(&format!("{dir}/basiclo-318018-04.bin"))?)
            .map_err(|e| e.to_string())?;
        let hi = std::fs::read(self.file(&format!("{dir}/basichi-318019-04.bin"))?)
            .map_err(|e| e.to_string())?;
        let basic = std::env::temp_dir().join(format!(
            "rc-suite-c128-basic-{}-{}.bin",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&basic, [lo, hi].concat()).map_err(|e| e.to_string())?;
        let path = |name: &str| -> Result<String, String> {
            Ok(self
                .file(&format!("{dir}/{name}"))?
                .to_string_lossy()
                .into_owned())
        };
        Ok(self
            .run("c128")
            .arg("--rom-basic")
            .arg(basic.to_string_lossy())
            .arg("--rom-kernal")
            .arg(path("kernal-318020-05.bin")?)
            .arg("--rom-char")
            .arg(path("chargen-390059-01.bin")?)
            .arg("--rom-c64-basic")
            .arg(path("basic64-901226-01.bin")?)
            .arg("--rom-c64-kernal")
            .arg(path("kernal64-901227-03.bin")?))
    }
}

/// The Commodore screen code of an ASCII character in the upper-case set: `@` is 0, `A`-`Z` are 1-26, and the characters
/// from space to `?` are their ASCII codes. Anything else maps to `None`.
pub fn screen_code(c: char) -> Option<u8> {
    match c {
        '@' => Some(0),
        'A'..='Z' => Some(c as u8 - b'A' + 1),
        ' '..='?' => Some(c as u8),
        _ => None,
    }
}

/// `text` as screen codes, `None` if a character has none.
pub fn screen_codes(text: &str) -> Option<Vec<u8>> {
    text.chars().map(screen_code).collect()
}

/// The offset of `text` (as screen codes) in `screen`, if it is there.
pub fn find_text(screen: &[u8], text: &str) -> Option<usize> {
    let codes = screen_codes(text)?;
    screen
        .windows(codes.len())
        .position(|w| w == codes.as_slice())
}

/// Frames a CPC firmware needs from reset to the BASIC prompt, with room to spare.
pub const CPC_BOOT_FRAMES: u32 = 250;

/// How a firmware-resident CPC run is set up: what goes on the command line besides the machine and its ROM
/// (a `--disk`, say), the snapshot version to resume from, and how long to run.
pub struct Resident<'a> {
    pub machine: &'a str,
    pub rom: &'a Path,
    /// Extra arguments for both the boot run and the resumed run (`--disk`, `foo.dsk`, ...).
    pub extra: Vec<String>,
    /// Write the patched snapshot as this `.sna` version (1, 2 or 3); `None` keeps the booted one.
    pub sna_version: Option<u8>,
}

impl Resident<'_> {
    /// Runs Z80 `source` (assembled at `org`) on a CPC whose firmware has booted to BASIC: the machine is
    /// booted for [`CPC_BOOT_FRAMES`] frames and saved as a snapshot, the code is written into its RAM and the
    /// program counter pointed at it, and the patched snapshot is resumed for `frames` frames. The firmware's
    /// jump block and its interrupts are live, so the code can call the firmware (AMSDOS, the keyboard, ...).
    pub fn run(
        &self,
        env: &Env,
        tag: &str,
        org: u16,
        source: &str,
        frames: u32,
        dumps: &[(u16, u16)],
    ) -> Result<Outcome, String> {
        let dir = std::env::temp_dir();
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let boot = dir.join(format!(
            "rc-suite-{tag}-{}-{unique}-boot.sna",
            std::process::id()
        ));
        let patched = dir.join(format!(
            "rc-suite-{tag}-{}-{unique}-run.sna",
            std::process::id()
        ));

        let mut booting = env.run(self.machine).rom(self.rom);
        for a in &self.extra {
            booting = booting.arg(a.clone());
        }
        booting
            .frames(CPC_BOOT_FRAMES)
            .arg("--dump-snapshot")
            .arg(boot.to_string_lossy())
            .go()?;

        let bytes = std::fs::read(&boot).map_err(|e| format!("no boot snapshot: {e}"))?;
        let mut snapshot =
            cpc_snapshot::parse(&bytes).map_err(|e| format!("boot snapshot: {e}"))?;
        let code = z80_image(org, source)?;
        let at = usize::from(org);
        snapshot
            .ram
            .get_mut(at..at + code.len())
            .ok_or("the program does not fit in the snapshot's RAM")?
            .copy_from_slice(&code);
        snapshot.cpu.pc = org;
        if let Some(v) = self.sna_version {
            snapshot.version = v;
        }
        let out = cpc_snapshot::write(&snapshot).map_err(|e| format!("patched snapshot: {e}"))?;
        std::fs::write(&patched, out).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&boot);

        let mut resumed = env.run(self.machine).rom(self.rom);
        for a in &self.extra {
            resumed = resumed.arg(a.clone());
        }
        resumed = resumed
            .arg("--snapshot")
            .arg(patched.to_string_lossy())
            .frames(frames);
        for (start, len) in dumps {
            resumed = resumed.dump(*start, *len);
        }
        let result = resumed.go();
        let _ = std::fs::remove_file(&patched);
        result
    }
}

#[cfg(test)]
mod audio_tests {
    use super::Audio;

    fn sine(freq: f64, rate: u32, seconds: f64) -> Audio {
        let n = (f64::from(rate) * seconds) as usize;
        let samples = (0..n)
            .map(|i| {
                (0.5 * (2.0 * std::f64::consts::PI * freq * i as f64 / f64::from(rate)).sin())
                    as f32
            })
            .collect();
        Audio {
            rate,
            samples,
            channels: 1,
        }
    }

    #[test]
    fn peak_finds_a_tone_whose_pitch_is_not_known_exactly() {
        let audio = sine(520.8, 44100, 2.0);
        // The exact-bin reading at the nominal 520 Hz misses most of it ...
        assert!(audio.tone(520.0, 0.5, 1.5) < 0.2);
        // ... the scan within 1 % finds it, close to its true pitch and amplitude.
        let (f, a) = audio.peak(515.0, 525.0, 0.5, 1.5);
        assert!((f - 520.8).abs() < 0.3, "found {f}");
        assert!((a - 0.5).abs() < 0.05, "amplitude {a}");
    }
}

#[cfg(test)]
mod screen_tests {
    use super::Screen;

    /// Hatari writes indexed-colour PNGs (colour type 3): they read back as their palette's colours, not as grey
    /// levels of the indices.
    #[test]
    fn an_indexed_png_reads_as_its_palette_colours() {
        let path =
            std::env::temp_dir().join(format!("rc-suite-indexed-{}.png", std::process::id()));
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut enc = png::Encoder::new(std::io::BufWriter::new(file), 2, 1);
            enc.set_color(png::ColorType::Indexed);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_palette(vec![0x00, 0x8F, 0x00, 0xFF, 0xFF, 0xFF]);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&[0, 1]).unwrap();
        }
        let s = Screen::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(s.pixel(0, 0), [0x00, 0x8F, 0x00]);
        assert_eq!(s.pixel(1, 0), [0xFF, 0xFF, 0xFF]);
    }
}
