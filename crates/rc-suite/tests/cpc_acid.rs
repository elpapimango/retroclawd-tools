//! CPC acid tests: Kevin Thacker's "Acid Tests Suite" (cpcwiki, from the Arnold emulator), the programs that judge themselves.
//!
//! Each program prints one line per test, `name-PASS` or `name-FAIL`, and ends with "Done. Press a key to exit"; the author ran
//! all of them on real machines ("PASS on real hardware", his readme). They are copied from a local folder by
//! `tools/fetch_acid.sh --from <Acid_Tests_Suite folder>` (never committed; the programs are free to distribute only without
//! modification) into `amstrad/software/acid/`.
//!
//! The programs are run on the disk the author built, through the firmware (`RUN"NAME`). What they print is read from the
//! machine itself: `--capture-at 0xBB5A:A=FILE` logs register A each time the CPU reaches the firmware's TXT OUTPUT entry, so the
//! whole transcript is there even when the text scrolls away. Their output is paged, so a run goes on in segments of 200 frames
//! through a save state, and Space is pressed only after a segment that printed nothing (a key pressed while a test runs would
//! change what the keyboard tests read).
//!
//! Every program has the outcome we expect today: which tests fail and whether it finishes. A test that now passes, or fails
//! newly, or a program that stops finishing, fails the suite; the lists are the known gaps, to be fixed or explained.
//!
//! Needs `RETROCLAWD_BIN`, `RETROCLAWD_MACHINE_PLUGIN_DIRS` and `RC_ROMS`; skips without, and for a program whose disk is absent.

use rc_suite::Env;
use std::collections::BTreeSet;

const ROM_6128: &str = "amstrad/roms/cpc/cpc6128.rom";
const FIRST_SEGMENT: u32 = 700;
const SEGMENT: u32 = 200;
/// Segments without output after which the program counts as hung.
const SILENT_LIMIT: u32 = 40;

struct Spec {
    name: &'static str,
    machine: &'static str,
    disk: &'static str,
    run: &'static str,
    /// Extra command-line arguments (`--crtc-type 1`).
    extra: &'static [&'static str],
    /// (text of a prompt, key to press when output stops after it; the prompt shown last wins) -- Space otherwise.
    answers: &'static [(&'static str, &'static str)],
    max_frames: u32,
}

const fn spec(
    name: &'static str,
    machine: &'static str,
    disk: &'static str,
    run: &'static str,
    max_frames: u32,
) -> Spec {
    Spec {
        name,
        machine,
        disk,
        run,
        extra: &[],
        answers: &[],
        max_frames,
    }
}

struct Outcome {
    finished: bool,
    passed: BTreeSet<String>,
    failed: BTreeSet<String>,
    last_line: String,
    frames: u32,
}

/// `name-PASS` / `name-FAIL` lines; a first test's name can be glued to the prompt before it.
fn results(transcript: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    for line in transcript.replace('\r', "\n").lines() {
        let line = line.trim_end();
        let Some((head, ok)) = line
            .strip_suffix("-PASS")
            .map(|h| (h, true))
            .or_else(|| line.strip_suffix("-FAIL").map(|h| (h, false)))
        else {
            continue;
        };
        if head.ends_with(' ') || head.is_empty() {
            continue; // "0009: got FF expected 09 - FAIL": a detail line, not a result
        }
        let head = head.strip_prefix("Press a key to start").unwrap_or(head);
        out.push((head.to_string(), ok));
    }
    out
}

fn transcript_of(env: &Env, spec: &Spec) -> Result<Outcome, String> {
    let rom = env.file(ROM_6128)?;
    let disk = env.file(spec.disk)?;
    let tmp = std::env::temp_dir();
    let tag = format!("rc-acid-{}-{}", std::process::id(), spec.name);
    let state = tmp.join(format!("{tag}.state"));
    let cap = tmp.join(format!("{tag}.cap"));
    let mut text = String::new();
    let mut frames = 0u32;
    let mut press = false;
    let mut silent = 0u32;
    let mut first = true;
    let outcome = loop {
        let len = if first { FIRST_SEGMENT } else { SEGMENT };
        let mut run = env
            .run(spec.machine)
            .rom(&rom)
            .arg("--disk")
            .arg(disk.to_string_lossy())
            .arg("--capture-at")
            .arg(format!("0xBB5A:A={}", cap.display()))
            .arg("--at")
            .arg(format!("{len}:save_state={}", state.display()));
        for e in spec.extra {
            run = run.arg(*e);
        }
        if first {
            run = run
                .arg("--type-after")
                .arg("150")
                .arg("--type-delay")
                .arg("80")
                .arg("--type")
                .arg(format!("RUN\"{}\n", spec.run));
        } else {
            run = run.arg("--load-state").arg(state.to_string_lossy());
        }
        if press {
            // The prompt shown last decides the key (several questions in a row each wait for one).
            let plain = text.replace('\r', "\n");
            let key = spec
                .answers
                .iter()
                .filter_map(|(t, k)| plain.rfind(t).map(|at| (at, *k)))
                .max_by_key(|(at, _)| *at)
                .map_or("Space", |(_, k)| k);
            run = run
                .arg("--at")
                .arg(format!("0:key_down={key}"))
                .arg("--at")
                .arg(format!("6:key_up={key}"));
        }
        run.frames(len).go()?;
        frames += len;
        first = false;
        let new =
            std::fs::read(&cap).map_err(|e| format!("no transcript {}: {e}", cap.display()))?;
        let new: String = new.iter().map(|&b| b as char).collect();
        press = new.is_empty();
        silent = if new.is_empty() { silent + 1 } else { 0 };
        text.push_str(&new);
        if text.contains("Done.") || frames >= spec.max_frames || silent >= SILENT_LIMIT {
            let list = results(&text);
            break Outcome {
                finished: text.contains("Done."),
                passed: list.iter().filter(|r| r.1).map(|r| r.0.clone()).collect(),
                failed: list.iter().filter(|r| !r.1).map(|r| r.0.clone()).collect(),
                last_line: text
                    .replace('\r', "\n")
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("")
                    .to_string(),
                frames,
            };
        }
    };
    let _ = std::fs::remove_file(&state);
    let _ = std::fs::remove_file(&cap);
    Ok(outcome)
}

/// `Ok(None)`: as expected and all passes; `Ok(Some(note))`: as expected, with known failures; `Err`: not what we expect.
fn check(env: &Env, spec: &Spec) -> Result<Option<String>, String> {
    let got = transcript_of(env, spec)?;
    if std::env::var_os("RC_ACID_PRINT").is_some() {
        eprintln!(
            "BASELINE|{}|{}|{:?}|{}|{:?}|{}",
            spec.name,
            got.finished,
            got.failed,
            got.passed.len(),
            got.last_line,
            got.frames
        );
    }
    let (finishes, failing) = BASELINE
        .iter()
        .find(|b| b.0 == spec.name)
        .map_or((true, &[][..]), |b| (b.1, b.2));
    let want: BTreeSet<String> = failing.iter().map(|s| s.to_string()).collect();
    let mut problems = Vec::new();
    if got.finished != finishes {
        problems.push(format!(
            "finishes: expected {}, got {} (last line {:?})",
            finishes, got.finished, got.last_line
        ));
    }
    for n in got.failed.difference(&want) {
        problems.push(format!("new failure: {n}"));
    }
    for n in want.difference(&got.failed) {
        problems.push(if got.passed.contains(n) {
            format!("now passes (remove from the list): {n}")
        } else {
            format!("expected failing test never ran: {n}")
        });
    }
    if !problems.is_empty() {
        return Err(problems.join("; "));
    }
    Ok(if got.finished && want.is_empty() {
        None
    } else {
        let tail = if got.finished {
            String::new()
        } else {
            format!("; does not finish (stops after {:?})", got.last_line)
        };
        Some(format!(
            "{} failing, {} passing{tail}",
            want.len(),
            got.passed.len()
        ))
    })
}

/// What each program does today (finishes, failing tests): the known gaps. Written from a run with `RC_ACID_PRINT=1`.
#[rustfmt::skip]
const BASELINE: &[(&str, bool, &[&str])] = &[
    ("ppi", true, &[
        "mode 0: port b control write reset test",
        "mode 0: port b output latch r/w",
        "mode 0: ppi port a output",
        "mode 0: psg control with bit set/reset",
        "mode 1: port A mode 1 (input), port B mode 0, port C bits input",
        "mode 1: port A mode 1 (input), port B mode 0, port C bits output",
        "mode 1: port A mode 1 (output), port B mode 0, port C bits input",
        "mode 1: port A mode 1 (output), port B mode 0, port C bits output",
        "mode 2: port A mode 2, port B mode 0,  port C bits output",
        "mode 2: port A mode 2, port B mode 0, port C bits input",
        "ppi port reset from control",
    ]),
    ("psg", true, &[
        "invalid register write test",
        "port A out r/w test",
        "port A w through test",
        "reg write/read mask test",
    ]),
    ("cpctest", false, &[
        "CPC int (HSYNC pos)",
        "CPC int (HSYNC width)",
    ]),
    ("romtest", true, &[
    ]),
    ("type1ctrl", true, &[
        "type 1 status (clear lpen)",
        "type 1 status (lpen)",
    ]),
    ("asicfloat", false, &[
        "asic ram float (5000-50ff)",
    ]),
    ("asiclock", true, &[
        "asic RMR2 lock",
    ]),
    ("asicppi", false, &[
        "mode 0: psg control with bit set/reset",
    ]),
    ("asicrom", true, &[
        "RMR2 (bits 2..0) - low rom cart page",
        "RMR2 configs (bit 3,4)",
    ]),
    ("asictest", false, &[
        "CPC border set (bit 5=0)",
        "CPC border set (bit 5=1)",
        "CPC colour set (bit 5=0)",
        "CPC colour set (bit 5=1)",
        "CPC mode set (bit 5=0) (locked)",
        "CPC mode set (bit 5=0) (unlocked)",
        "CPC mode set (bit 5=1) (locked)",
        "CPC mode set (rmr2) (bit 5=1) (unlocked)",
        "CPC pen set (bit 5=0)",
        "CPC pen set (bit 5=1)",
        "DCSR mirror",
        "IN set colour",
        "sprite coord test y (l/h)",
        "sprite ram mask",
    ]),
    ("asic_external_ram", true, &[
        "write through c4 reg",
        "write through c4 unmapped",
    ]),
    ("dmatest", false, &[
        "dcsr read/write irq",
        "dma enable test (dcsr bits)",
        "dma int request test (dcsr bits)",
    ]),
    ("pritest", false, &[
        "6c0f PRI status",
        "PRI 2 lines into vsync (R4 fixed, R7 varies)",
        "PRI all values - count lines as expected - R7>PRI",
        "PRI can delay normal raster interrupt",
        "PRI re-trigger (hsync pos)",
        "PRI start (R2)",
        "PRI start (R3)",
        "PRI trigger bug (hsync pos)",
        "PRI trigger bug (hsync width)",
        "PRI triggering tests",
        "pri all values",
        "pri all values (through vsync)",
        "pri repeat (r9>8)",
        "pri wrap",
    ]),
];

const D: &str = "amstrad/software/acid";

fn specs() -> Vec<Spec> {
    let d = |rest: &str| -> &'static str { Box::leak(format!("{D}/{rest}").into_boxed_str()) };
    vec![
        spec("ppi", "cpc6128", d("ppi/ppi.dsk"), "PPI", 80_000),
        spec("psg", "cpc6128", d("psg/psg.dsk"), "PSG", 120_000),
        spec("cpctest", "cpc6128", d("cpc/cpc.dsk"), "CPCTEST", 60_000),
        Spec {
            answers: &[("Choice", "Digit3")],
            ..spec(
                "romtest",
                "cpc6128",
                d("cpcrom/romtest.dsk"),
                "ROMTEST",
                20_000,
            )
        },
        Spec {
            extra: &["--crtc-type", "1"],
            ..spec(
                "type1ctrl",
                "cpc6128",
                d("crtc/crtc2.dsk"),
                "TYPE1CTR",
                20_000,
            )
        },
        spec(
            "asicfloat",
            "cpc6128plus",
            d("plus/asic1.dsk"),
            "FLOAT",
            30_000,
        ),
        spec(
            "asiclock",
            "cpc6128plus",
            d("plus/asic1.dsk"),
            "LOCK",
            20_000,
        ),
        spec("asicppi", "cpc6128plus", d("plus/asic1.dsk"), "PPI", 60_000),
        spec("asicrom", "cpc6128plus", d("plus/asic1.dsk"), "ROM", 20_000),
        spec(
            "asictest",
            "cpc6128plus",
            d("plus/asic1.dsk"),
            "TEST",
            60_000,
        ),
        spec(
            "asic_external_ram",
            "cpc6128plus",
            d("plus/asic1.dsk"),
            "EXTNRAM",
            20_000,
        ),
        spec(
            "dmatest",
            "cpc6128plus",
            d("plus/asic1.dsk"),
            "DMATEST",
            60_000,
        ),
        spec(
            "pritest",
            "cpc6128plus",
            d("plus/asic1.dsk"),
            "PRITEST",
            60_000,
        ),
    ]
}

#[test]
fn cpc_acid_tests() {
    let Some(env) = Env::discover() else {
        eprintln!("skipping: RETROCLAWD_BIN / RC_ROMS not set");
        return;
    };
    let only = std::env::var("RC_ACID_ONLY").ok();
    let all = specs();
    let runnable: Vec<&Spec> = all
        .iter()
        .filter(|s| only.as_deref().is_none_or(|o| o == s.name))
        .filter(|s| {
            let present = env.file(s.disk).is_ok();
            if !present {
                eprintln!(
                    "skipping {}: {} not present (tools/fetch_acid.sh --from <Acid_Tests_Suite folder> copies it)",
                    s.name, s.disk
                );
            }
            present
        })
        .collect();
    let results: Vec<(&str, Result<Option<String>, String>)> = std::thread::scope(|scope| {
        let env = &env;
        let handles: Vec<_> = runnable
            .iter()
            .map(|s| (s.name, scope.spawn(move || check(env, s))))
            .collect();
        handles
            .into_iter()
            .map(|(n, h)| (n, h.join().unwrap_or_else(|_| Err("panicked".to_string()))))
            .collect()
    });
    let mut failed = false;
    for (name, r) in &results {
        match r {
            Ok(None) => eprintln!("  pass   {name}"),
            Ok(Some(note)) => eprintln!("  gap    {name}: {note}"),
            Err(e) => {
                failed = true;
                eprintln!("  FAIL   {name}: {e}");
            }
        }
    }
    assert!(
        !failed,
        "a CPC acid test is not in its expected state (see above)"
    );
}
