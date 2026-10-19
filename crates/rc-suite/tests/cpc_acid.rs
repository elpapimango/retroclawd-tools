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
//! `RC_ACID_ONLY=<name>` runs one program, `RC_ACID_PRINT=1` prints each program's baseline line and
//! `RC_ACID_TRANSCRIPT=<folder>` keeps each transcript there.

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
    /// From the moment this text is on the screen Space is held down for the rest of the run (empty: never): the
    /// program's own paging then runs at the keyboard's repeat rate instead of stalling for a press per page. Only
    /// for programs that do not read the keyboard matrix themselves.
    hold_after: &'static str,
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
        hold_after: "",
    }
}

struct Outcome {
    finished: bool,
    passed: BTreeSet<String>,
    failed: BTreeSet<String>,
    last_line: String,
    frames: u32,
}

/// A name with every run of one repeated character squeezed to one. The transcript is taken each time the CPU
/// reaches the firmware's TXT OUTPUT entry, and an interrupt taken exactly there makes the entry count twice, so
/// a character can appear doubled ("throughh"): names are compared without such runs.
fn squeezed(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if !out.ends_with(c) {
            out.push(c);
        }
    }
    out
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
        // A first test's name can be glued to the prompt printed before it.
        let head = [
            "Press a key to start",
            "Press a key to continue",
            "Press a keey to continue",
        ]
        .iter()
        .find_map(|prompt| head.strip_prefix(prompt))
        .unwrap_or(head);
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
    let mut held = false;
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
        if !spec.hold_after.is_empty() && !held && text.contains(spec.hold_after) {
            held = true;
            run = run.arg("--at").arg("0:key_down=Space");
        } else if press && !held {
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
        let done = squeezed(&text).contains("Done.");
        if done || frames >= spec.max_frames || silent >= SILENT_LIMIT {
            let list = results(&text);
            break Outcome {
                finished: done,
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
    // `RC_ACID_TRANSCRIPT=<folder>`: keep each program's whole transcript (every "got .. expected .." line) as
    // `<folder>/<name>.txt`, for working on a failure.
    if let Some(dir) = std::env::var_os("RC_ACID_TRANSCRIPT") {
        let path = std::path::Path::new(&dir).join(format!("{}.txt", spec.name));
        std::fs::write(&path, text.replace('\r', "\n"))
            .map_err(|e| format!("writing {}: {e}", path.display()))?;
    }
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
    let mut got = got;
    got.passed = got.passed.iter().map(|n| squeezed(n)).collect();
    got.failed = got.failed.iter().map(|n| squeezed(n)).collect();
    // Tests the author himself marks as unreliable on real hardware ("todo: fix, vsync gets in the way"): the
    // result depends on where in the frame the machine happens to be, so neither outcome is a finding.
    for (program, test) in UNRELIABLE {
        if *program == spec.name {
            got.passed.remove(&squeezed(test));
            got.failed.remove(&squeezed(test));
        }
    }
    let want: BTreeSet<String> = failing.iter().map(|s| squeezed(s)).collect();
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
    ]),
    ("psg", true, &[
    ]),
    // The GA I/O decode test never loads BC before `io_decode` stores it as its restore hook (`ga_io_restore_extra`
    // exists but is unused), so it calls a stale address and resets the machine: a bug in the test.
    ("cpctest", false, &[
    ]),
    ("romtest", true, &[
    ]),
    ("type1ctrl", true, &[
    ]),
    ("asicfloat", true, &[
    ]),
    ("asiclock", true, &[
    ]),
    ("asicppi", true, &[
    ]),
    ("asicrom", true, &[
    ]),
    ("asictest", true, &[
    ]),
    ("asic_external_ram", true, &[
    ]),
    // "dma under asic registers" prints every value as expected but reports FAIL depending on the frame phase the
    // program starts at (it passed while the program loaded from an instant disk).
    ("dmatest", true, &[
        "CRTC R0 length and dma",
        "dma int request test (dcsr bits)",
        "dma pause (change prescale higher)",
        "dma pause (change prescale lower)",
        "dma under asic registers",
    ]),
    ("pritest", true, &[
        "PRI (enable norm during pri int handler)",
        "PRI start (R2)",
        "im2 vector (norm)",
    ]),
    // "ready change, sense interrupt status, ..." has no expected values of its own (its table is only the end mark):
    // it compares with what the previous test left in the buffer.
    ("fdc_gen", true, &[
        "read id not ready",
        "ready change, sense interrupt status, ready change, sense interrupt status (drive 0 best)",
    ]),
    ("fdc_hang", true, &[
    ]),
    ("fdc_lowlevel", true, &[
        "9 SC, Format N=1, ID N=3",
        "9 SC, Format N=2, ID N=2, Write GPL>FFormat GPL",
    ]),
    // The program marks sector &41 of track 0 deleted while initialising, yet its track-0 tables ("read data", the
    // overrun cases) expect a normal data mark there; we read the deleted mark it wrote.
    // The bad cylinder, R=EOT=ff, eot and skip tests overwrite HL (their expected-results pointer) before saving it,
    // so they compare with the previous test's buffer.
    ("fdc_read", true, &[
        "read data",
        "read data  (R=EOT=ff)",
        "read data (bad cylinder/wrong cylinder - data)",
        "read data (bad cylinder/wrong cylinder - deleted data)",
        "read data  (data error)",
        "read data (eot)",
        "read data (no polling status register)",
        "read data (overrun - data error on data)",
        "read data (overrun - data)",
        "read data (ready change - data crc error on data)",
        "read data (ready change - deleted data)",
        "read data (ready change after 1st byte of sector)",
        "read data (ready change before first sector)",
        "read data then sense int",
        "read data x bytes and skip",
        "read deleted data (R=EOT=ff)",
        "read deleted data (bad cylinder/wrong cylinder - data)",
        "read deleted data (bad cylinder/wrong cylinder - deleted data)",
        "read deleted data (data error)",
        "read deleted data (eot)",
        "read deleted data (no polling status  register)",
        "read deletted data (overrun - data error on data)",
        "read deleted data (overrun - data)",
        "read deleted data (overrun - deleted data)",
        "read deleted data (ready change - data crc error)",
        "read deleted data (ready change - data)",
        "read deleted data (ready change after 1st byte of sector)",
        "read deletted data (ready change before first sector)",
    ]),
    ("fdc_seek", true, &[
        "recal (not ready - not trk0)",
        "recal (reaady change)",
        "recal command repeat (while seeking)",
        "recal then seek",
        "seek (ready change)",
        "seek (try read id during seek)",
        "seek command repeat (while seeking)",
        "seek drive n and drive n+2 (n->0, n+2 -> 39) (wait first seek)",
        "seek drive n and drive n+2 (n->10, n+2 -> 10)",
        "seek then invalid (&ff) command",
    ]),
    ("fdc_time", true, &[
        "read data timing (cmd->execution phase start)",
        "read data timing (data end->execution phase end)",
        "read data timing (data end->result phase)",
        "read data timing (execution phase end->result phase)",
        "seek (fdc busy to not busy after seek command sent)",
        "write data timing (data end->execution phase end)",
        "write data timing (data end->result phase)",
        "write data timing (executtion phase end->result phase)",
    ]),
    // Needs the disc write-protected by hand mid-run.
    ("fdc_wp", false, &[
    ]),
    // "write deleted data (dma)" expects an overrun on track 10 while "write data (dma)" (track 0) expects a normal end
    // from the same DMA set-up; the model ends both normally.
    ("fdc_write", true, &[
        "write data (bad cylinder)",
        "write data (change c=ff?)",
        "write data (data error - remove)",
        "write dataa (fm on fm)",
        "write data (fm on mfm)",
        "write data (mfm on fm)",
        "write data (wrong cylinder)",
        "write deleted data (bad cylinder)",
        "write deleeted data (change c=ff?)",
        "write deleted data (data error - remove)",
        "write deleted data (dma)",
        "write deleted data (fm on fm)",
        "write deleted data (fm on mfm)",
        "write deleted data (mfm on fm)",
    ]),
    // Runs past the frame limit in "format id/read track (all N)".
    ("fdctest3", false, &[
        "format (force data error)",
        "format (not ready)",
        "format (ss)",
        "read sector (using write data register)",
        "write sector (using read data_register)",
    ]),
];

/// (program, test) pairs whose outcome is no finding about the emulator:
/// - `ppi` "port b read (vsync on)": the author marks it unreliable (`ppi.asm`: "todo: fix, vsync gets in the way and
///   sometimes cassette write"); the vsync-on read of Port B depends on the frame phase the earlier tests leave.
/// - `asictest` "sprite ram mask": its 8K result buffer (from the end of the program, `&972A`) runs to `&B72A`, into the
///   firmware's variables; after `EI` the firmware's interrupt handler writes there before the last results are
///   compared, so entries `&FC0` on read back garbage. Which bytes depends on the firmware (this suite runs the classic
///   6128 ROM, not the 6128 Plus's own); every entry before them passes.
const UNRELIABLE: &[(&str, &str)] = &[
    ("ppi", "mode 0: port b read (vsync on)"),
    ("asictest", "sprite ram mask"),
];

const D: &str = "amstrad/software/acid";

/// The disc programs ask for the FDC (NEC D765AC-2), the /DACK wiring (CPC6128), the drive configuration (two drives
/// at most), whether several drives may be used at once (no), the main drive (0), the drive type (40 tracks) and the
/// ready speed (slow, a 3 inch drive); after "Testing drive" Space is held to page through their long output.
const FDC_ANSWERS: &[(&str, &str)] = &[
    ("UM8272", "Digit2"),
    ("6128Plus, 6128 cost down", "Digit1"),
    ("Four drives MAX", "Digit1"),
    ("the main drive and other drives", "KeyN"),
    ("Choose drive (0,1,2 or 3)", "Digit0"),
    ("80 track double sided", "Digit1"),
    ("2: Fast (e.g. 3.5inch", "Digit1"),
];

fn fdc(name: &'static str, disk: &'static str) -> Spec {
    Spec {
        answers: FDC_ANSWERS,
        hold_after: "Testing drive",
        ..spec(name, "cpc6128", disk, "FDCTEST.BIN", 60_000)
    }
}

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
            120_000,
        ),
        // The program asks what RAM the machine has: a 6128Plus has 128K, no expansion (its own list: "Y then N").
        Spec {
            answers: &[
                ("Has internal extra 64KB", "KeyY"),
                ("Has external extra 64KB", "KeyN"),
            ],
            ..spec(
                "asic_external_ram",
                "cpc6128plus",
                d("plus/asic1.dsk"),
                "EXTNRAM",
                20_000,
            )
        },
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
        fdc("fdc_gen", d("disc/fdc_gen/cpc_fdctest.dsk")),
        fdc("fdc_hang", d("disc/fdc_hang/cpc_fdctest.dsk")),
        fdc("fdc_lowlevel", d("disc/fdc_lowlevel/cpc_fdctest.dsk")),
        fdc("fdc_read", d("disc/fdc_read/cpc_fdctest.dsk")),
        fdc("fdc_seek", d("disc/fdc_seek/cpc_fdctest.dsk")),
        fdc("fdc_time", d("disc/fdc_time/cpc_fdctest.dsk")),
        fdc("fdc_wp", d("disc/fdc_wp/cpc_fdctest.dsk")),
        fdc("fdc_write", d("disc/fdc_write/cpc_fdctest.dsk")),
        fdc("fdctest3", d("disc/fdctest3/cpc_fdctest.dsk")),
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
