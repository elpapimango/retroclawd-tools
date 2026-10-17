//! `rcld` as a program.

use std::path::PathBuf;
use std::process::{Command, Output};

fn rcld(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rcld"))
        .args(args)
        .output()
        .expect("runs rcld")
}

fn ok(args: &[&str]) -> (String, String) {
    let out = rcld(args);
    assert!(
        out.status.success(),
        "rcld {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn fails(args: &[&str]) -> String {
    let out = rcld(args);
    assert_eq!(
        out.status.code(),
        Some(2),
        "rcld {args:?} should fail with status 2"
    );
    String::from_utf8_lossy(&out.stderr).into_owned()
}

struct Temp(PathBuf);

impl Temp {
    fn new(tag: &str) -> Temp {
        let dir = std::env::temp_dir().join(format!("rcld-cli-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Temp(dir)
    }
    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
    fn write(&self, name: &str, text: &str) -> String {
        std::fs::write(self.0.join(name), text).unwrap();
        self.path(name)
    }
    fn read(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.0.join(name)).unwrap()
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const LAYOUT: &str = "machine = \"c64\"\nname = \"HELLO\"\nentry = \"start\"\noutput = \"out/hello.prg\"\n[[input]]\nasm = \"src/main.asm\"\n[[region]]\nname = \"ram\"\nstart = 0x0801\nend = 0x9FFF\n";

fn project(t: &Temp) -> String {
    std::fs::create_dir_all(t.0.join("src")).unwrap();
    std::fs::create_dir_all(t.0.join("out")).unwrap();
    t.write("src/main.asm", "* = $080D\nstart: inc $D020\n jmp start\n");
    t.write("layout.toml", LAYOUT)
}

#[test]
fn build_takes_paths_relative_to_the_layout_and_writes_the_extras() {
    let t = Temp::new("build");
    let layout = project(&t);
    // Run from another directory: everything is relative to the layout file.
    let out = Command::new(env!("CARGO_BIN_EXE_rcld"))
        .args([
            "build",
            &layout,
            "--map",
            &t.path("hello.map"),
            "--symbols",
            &t.path("hello.sym"),
        ])
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("note: added the BASIC line 10 SYS 2061"),
        "{stderr}"
    );
    let prg = t.read("out/hello.prg");
    assert_eq!(prg[..2], [0x01, 0x08]);
    assert_eq!(prg[2 + 12..], [0xEE, 0x20, 0xD0, 0x4C, 0x0D, 0x08]);
    let map = String::from_utf8(t.read("hello.map")).unwrap();
    assert!(
        map.contains("entry: $080D (start)") && map.contains("ram"),
        "{map}"
    );
    assert_eq!(
        String::from_utf8(t.read("hello.sym")).unwrap(),
        "start = $080D\n"
    );
}

fn the_extension_chooses_the_image_and_nothing_is_overwritten() {
    let t = Temp::new("force");
    let layout = project(&t);
    let d64 = t.path("game.d64");
    ok(&["build", &layout, "-o", &d64]);
    assert!(t.read("game.d64").len() == 174848);
    assert!(fails(&["build", &layout, "-o", &d64]).contains("--force"));
    ok(&["build", &layout, "-o", &d64, "--force"]);
    assert!(
        fails(&["build", &layout, "-o", &t.path("game.tap")])
            .contains("cannot be written as \".tap\"")
    );
}

fn defines_and_errors() {
    let t = Temp::new("errors");
    let layout = project(&t);
    t.write("src/main.asm", "* = $080D\nstart: lda #LEVEL\n jmp start\n");
    ok(&["build", &layout, "-D", "LEVEL=$20", "--force"]);
    assert_eq!(t.read("out/hello.prg")[2 + 12 + 1], 0x20);
    assert!(
        fails(&["build", &layout, "--force"]).contains("main.asm"),
        "an undefined constant names its file"
    );
    assert!(fails(&["build", &layout, "-D", "LEVEL"]).contains("NAME=VALUE"));
    assert!(fails(&["build", &t.path("missing.toml")]).contains("cannot read"));
    assert!(fails(&["build", &t.write("bad.toml", "machine = [")]).contains("bad.toml"));
    assert!(fails(&["build", &t.write("noout.toml", "machine = \"c64\"\n")]).contains("no output"));
}
/// Merged (every check kept): `defines_and_errors`, `the_extension_chooses_the_image_and_nothing_is_overwritten`.
#[test]
fn defines_and_images() {
    defines_and_errors();
    the_extension_chooses_the_image_and_nothing_is_overwritten();
}
