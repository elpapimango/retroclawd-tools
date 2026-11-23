use rc_common::obj::{Archive, Object};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("rcar-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Dir(d)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    /// Assembles `source` into `name`.rco in the directory.
    fn object(&self, name: &str, source: &str) -> PathBuf {
        let opts = rcasm::Options {
            cpu: Some(rcasm::Cpu::Mos6502),
            ..rcasm::Options::default()
        };
        let o = rcasm::assemble_object(name, source, &opts, &rcasm::DiskFiles)
            .unwrap_or_else(|e| panic!("{e:?}"));
        let p = self.path(&format!("{name}.rco"));
        std::fs::write(&p, o.encode()).unwrap();
        p
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn rcar(args: &[&Path]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rcar"))
        .args(args)
        .output()
        .unwrap()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn p(s: &str) -> &Path {
    Path::new(s)
}

#[test]
fn create_list_extract_add_delete() {
    let d = Dir::new("all");
    let a = d.object("a", ".export fa\n.import fb\nfa: jsr fb\n rts\n");
    let b = d.object("b", ".export fb\nfb: rts\n");
    let lib = d.path("l.rca");
    let out = rcar(&[p("create"), &lib, &a, &b]);
    assert!(out.status.success(), "{}", text(&out));
    let archive = Archive::decode(&std::fs::read(&lib).unwrap()).unwrap();
    assert_eq!(archive.len(), 2);
    assert_eq!(archive.find_export("fb").unwrap(), Some(1));

    let out = rcar(&[p("list"), &lib, p("--symbols")]);
    let t = text(&out);
    assert!(
        t.contains("a.rco") && t.contains("exports: fa") && t.contains("imports: fb"),
        "{t}"
    );

    let dir = d.path("out");
    std::fs::create_dir_all(&dir).unwrap();
    let out = rcar(&[p("extract"), &lib, p("b.rco"), p("-d"), &dir]);
    assert!(out.status.success(), "{}", text(&out));
    let b_bytes = std::fs::read(dir.join("b.rco")).unwrap();
    assert_eq!(b_bytes, std::fs::read(&b).unwrap());
    assert!(Object::decode(&b_bytes).is_ok());
    assert!(!dir.join("a.rco").exists());

    let c = d.object("c", ".export fc\nfc: rts\n");
    assert!(rcar(&[p("add"), &lib, &c]).status.success());
    assert_eq!(rcar_len(&lib), 3);
    assert!(rcar(&[p("delete"), &lib, p("a.rco")]).status.success());
    assert_eq!(rcar_len(&lib), 2);
    let out = rcar(&[p("delete"), &lib, p("a.rco")]);
    assert!(
        !out.status.success() && text(&out).contains("no member"),
        "{}",
        text(&out)
    );
}

fn rcar_len(lib: &Path) -> usize {
    Archive::decode(&std::fs::read(lib).unwrap()).unwrap().len()
}

#[test]
fn create_will_not_overwrite_without_force() {
    let d = Dir::new("force");
    let a = d.object("a", ".export fa\nfa: rts\n");
    let lib = d.path("l.rca");
    assert!(rcar(&[p("create"), &lib, &a]).status.success());
    let out = rcar(&[p("create"), &lib, &a]);
    assert!(!out.status.success());
    assert!(
        rcar(&[p("create"), &lib, &a, p("--force")])
            .status
            .success()
    );
}

#[test]
fn add_replaces_a_member_of_the_same_name() {
    let d = Dir::new("replace");
    let a = d.object("a", ".export one\none: rts\n");
    let lib = d.path("l.rca");
    assert!(rcar(&[p("add"), &lib, &a]).status.success());
    let a2 = d.object("a", ".export two\ntwo: rts\n");
    assert_eq!(a2, a);
    assert!(rcar(&[p("add"), &lib, &a2]).status.success());
    let archive = Archive::decode(&std::fs::read(&lib).unwrap()).unwrap();
    assert_eq!(archive.len(), 1);
    assert_eq!(archive.find_export("two").unwrap(), Some(0));
    assert_eq!(archive.find_export("one").unwrap(), None);
}

#[test]
fn a_file_that_is_not_an_object_is_an_error_naming_it() {
    let d = Dir::new("bad");
    let junk = d.path("junk.rco");
    std::fs::write(&junk, b"not an object").unwrap();
    let out = rcar(&[p("create"), &d.path("l.rca"), &junk]);
    assert!(!out.status.success());
    assert!(text(&out).contains("junk.rco"), "{}", text(&out));
    let out = rcar(&[p("list"), &junk]);
    assert!(!out.status.success());
}
