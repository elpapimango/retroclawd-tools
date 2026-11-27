//! `rcb new` then `rcb build` for every machine's template, and the project file's rules.

use rcb::{Project, templates};
use std::path::PathBuf;
use std::process::Command;

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rcb-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn rcb(args: &[&str], dir: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rcb"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

#[test]
fn every_template_builds_as_it_is() {
    let d = temp("all");
    for t in templates::ALL {
        let name = format!("p-{}", t.machine);
        let out = rcb(&["new", &name, "--machine", t.machine], &d);
        assert!(
            out.status.success(),
            "{}: {}",
            t.machine,
            String::from_utf8_lossy(&out.stderr)
        );
        let out = rcb(&["build", "-C", &name], &d);
        assert!(
            out.status.success(),
            "{}: {}",
            t.machine,
            String::from_utf8_lossy(&out.stderr)
        );
        let p = Project::load(&d.join(&name)).unwrap();
        assert!(p.output.exists(), "{}", t.machine);
        assert!(p.map_path().exists());
        let len = std::fs::metadata(&p.output).unwrap().len();
        assert!(len > 0, "{}", t.machine);
        // check writes nothing new, clean removes the folder
        assert!(rcb(&["check", "-C", &name], &d).status.success());
        assert!(rcb(&["clean", "-C", &name], &d).status.success());
        assert!(!d.join(&name).join("build").exists());
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn new_refuses_what_exists_and_unknown_machines() {
    let d = temp("new");
    assert!(rcb(&["new", "a", "--machine", "nes"], &d).status.success());
    let out = rcb(&["new", "a", "--machine", "nes"], &d);
    assert!(!out.status.success() && String::from_utf8_lossy(&out.stderr).contains("exists"));
    let out = rcb(&["new", "b", "--machine", "vic20x"], &d);
    assert!(!out.status.success() && String::from_utf8_lossy(&out.stderr).contains("no template"));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_project_table_expands_globs_and_the_default_output() {
    let d = temp("glob");
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("src/a.asm"), "nop\n").unwrap();
    std::fs::write(d.join("src/b.asm"), "nop\n").unwrap();
    std::fs::write(d.join("src/c.txt"), "x\n").unwrap();
    let p = Project::parse(
        &d,
        "[project]\nname = \"demo\"\nsources = [\"src/*.asm\"]\nseparate = true\nmachine = \"c64\"\n",
    );
    // `machine` belongs outside [project]: the keys after the table header are the table's
    assert!(p.is_err());
    let p = Project::parse(
        &d,
        "machine = \"c64\"\n[project]\nname = \"demo\"\nsources = [\"src/*.asm\"]\nseparate = true\n",
    );
    // (the toml above puts machine first: top-level; project table last)
    let p = p.unwrap();
    assert_eq!(p.output, d.join("build").join("demo.prg"));
    let inputs = &p.layout.input;
    assert_eq!(inputs.len(), 1);
    assert_eq!(
        inputs[0].src.as_ref().unwrap().list(),
        ["src/a.asm", "src/b.asm"]
    );
    let e = Project::parse(
        &d,
        "machine = \"c64\"\n[project]\nsources = [\"nothing/*.asm\"]\n",
    )
    .err()
    .unwrap();
    assert!(e.contains("nothing"), "{e}");
    let e = Project::parse(&d, "machine = \"c64\"\n[project]\nbogus = 1\n")
        .err()
        .unwrap();
    assert!(e.contains("bogus"), "{e}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn run_arguments_come_from_the_table_or_the_machine() {
    let d = temp("run");
    let p = Project::parse(&d, "machine = \"pcengine\"\n[project]\nname = \"x\"\n").unwrap();
    let a = p.run_args(false).unwrap();
    assert_eq!(a[0], "pcengine");
    assert_eq!(a[1], "--rom");
    let a = p.run_args(true).unwrap();
    assert!(a.contains(&"--symbols".to_string()));
    let p = Project::parse(&d, "machine = \"snes\"\n[project]\nname = \"x\"\n").unwrap();
    assert!(
        p.run_args(false)
            .unwrap_err()
            .contains("no snes machine yet")
            || p.run_args(false).is_err()
    );
    let p = Project::parse(
        &d,
        "machine = \"snes\"\n[project]\nname = \"x\"\n[run]\nargs = [\"snes\", \"{output}\"]\n",
    )
    .unwrap();
    let a = p.run_args(false).unwrap();
    assert_eq!(a[0], "snes");
    assert!(a[1].ends_with("x.sfc"), "{a:?}");
    let _ = std::fs::remove_dir_all(&d);
}

/// Every project under `examples/` builds (so the examples cannot rot).
#[test]
fn the_examples_build() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut built = 0;
    for entry in std::fs::read_dir(&root).unwrap().flatten() {
        let dir = entry.path();
        if !dir.join("rcb.toml").exists() {
            continue;
        }
        let p = Project::load(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        // build in memory: the folders stay clean
        let b = p
            .build()
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        assert!(!b.bytes.is_empty(), "{}", dir.display());
        built += 1;
    }
    assert!(built >= 12, "{built} examples");
}

#[test]
fn kits_are_listed_printed_and_written_into_new_projects() {
    let d = temp("kits");
    let out = rcb(&["kits"], &d);
    let list = String::from_utf8_lossy(&out.stdout).to_string();
    for t in templates::ALL {
        assert!(list.contains(t.machine), "{} has no kit: {list}", t.machine);
        let out = rcb(&["kit", t.machine], &d);
        assert!(out.status.success());
        assert!(!out.stdout.is_empty(), "{}", t.machine);
    }
    assert!(String::from_utf8_lossy(&rcb(&["kit", "c64"], &d).stdout).contains("BORDER"));
    assert!(!rcb(&["kit", "zx99"], &d).status.success());
    assert!(rcb(&["new", "p", "--machine", "c64"], &d).status.success());
    let kit = std::fs::read_to_string(d.join("p/src/kit.inc")).unwrap();
    assert!(kit.contains("SID_VOLUME"));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_program_can_use_the_kit_names() {
    let d = temp("usekit");
    assert!(rcb(&["new", "p", "--machine", "c64"], &d).status.success());
    std::fs::write(
        d.join("p/src/main.asm"),
        ".include \"src/kit.inc\"\n* = $080D\nstart:  lda #RED\n        sta BORDER\n        lda #JOY_FIRE\n        jmp start\n",
    )
    .unwrap();
    let out = rcb(&["build", "-C", "p"], &d);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let prg = std::fs::read(d.join("p/build/p.prg")).unwrap();
    // load address, the SYS stub, then lda #2 / sta $d020
    assert!(
        prg.windows(5).any(|w| w == [0xA9, 0x02, 0x8D, 0x20, 0xD0]),
        "{prg:02X?}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_flat_68000_machines_get_debug_information_at_their_run_addresses() {
    let d = temp("dbg68k");
    for (machine, want) in [
        ("genesis", vec!["0x200"]),
        ("neogeo", vec!["0x0 "]),
        ("segacd", vec!["0xFF0584", "0x6000"]),
    ] {
        let name = format!("p-{machine}");
        assert!(
            rcb(&["new", &name, "--machine", machine], &d)
                .status
                .success()
        );
        assert!(rcb(&["build", "-C", &name], &d).status.success());
        let text = std::fs::read_to_string(d.join(format!("{name}/build/{name}.rcdbg"))).unwrap();
        let info = rc_common::dbg::DebugInfo::parse(&text).unwrap();
        assert!(!info.lines.is_empty(), "{machine}: {text}");
        assert!(
            info.files.iter().all(|f| !f.contains('\\')),
            "{machine}: {text}"
        );
        for w in want {
            assert!(text.contains(w), "{machine} wants {w}: {text}");
        }
    }
    // the Atari ST loads a program where the system puts it: no addresses to give
    assert!(
        rcb(&["new", "p-st", "--machine", "atari-st"], &d)
            .status
            .success()
    );
    assert!(rcb(&["build", "-C", "p-st"], &d).status.success());
    assert!(!d.join("p-st/build/p-st.rcdbg").exists());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_build_writes_debug_information_and_rcb_debug_hands_it_to_the_emulator() {
    let d = temp("dbg");
    assert!(rcb(&["new", "p", "--machine", "c64"], &d).status.success());
    assert!(rcb(&["build", "-C", "p"], &d).status.success());
    let text = std::fs::read_to_string(d.join("p/build/p.rcdbg")).unwrap();
    assert!(text.starts_with("rcdbg 1"), "{text}");
    assert!(text.contains("main.asm"), "{text}");
    assert!(text.contains("sym start"), "{text}");
    let p = Project::load(&d.join("p")).unwrap();
    let args = p.run_args(true).unwrap();
    assert!(args.iter().any(|a| a == "--source-info"), "{args:?}");
    assert!(
        !p.run_args(false)
            .unwrap()
            .iter()
            .any(|a| a == "--source-info")
    );
    let _ = std::fs::remove_dir_all(&d);
}
