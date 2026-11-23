//! Linking objects and libraries: separate assembly, archives, placement rules and relocation checks.

use rc_common::obj::{Archive, Object};
use rcld::layout;
use std::path::{Path, PathBuf};

struct Project(PathBuf);

impl Project {
    fn new(tag: &str) -> Project {
        let dir = std::env::temp_dir().join(format!("rcld-link-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Project(dir)
    }
    fn file(&self, name: &str, text: &str) -> &Project {
        std::fs::write(self.0.join(name), text).unwrap();
        self
    }
    fn path(&self) -> &Path {
        &self.0
    }
    fn build(&self, layout_text: &str, out: &str) -> Result<rcld::Build, String> {
        let layout = layout::parse(layout_text)?;
        rcld::build(&layout, &self.0, &self.0.join(out), &[])
    }
    /// Assembles `name` into an object (6502).
    fn object(&self, name: &str) -> Object {
        let text = std::fs::read_to_string(self.0.join(name)).unwrap();
        let opts = rcasm::Options {
            cpu: Some(rcasm::Cpu::Mos6502),
            ..rcasm::Options::default()
        };
        rcasm::assemble_object(name, &text, &opts, &rcasm::DiskFiles)
            .unwrap_or_else(|e| panic!("{e:?}"))
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const MAIN: &str = "\
.import putc, msg
.export start
start:  ldx #0
loop:   lda msg,x
        beq done
        jsr putc
        inx
        bne loop
done:   rts
";

const LIB: &str = "\
.export putc, msg
putc:   sta $0400
        rts
.segment \"RODATA\"
msg:    .byte 72, 73, 0
";

const WHOLE: &str = "\
* = $0801
start:  ldx #0
loop:   lda msg,x
        beq done
        jsr putc
        inx
        bne loop
done:   rts
putc:   sta $0400
        rts
msg:    .byte 72, 73, 0
";

fn layout_with(inputs: &str, extra: &str) -> String {
    format!("machine = \"generic\"\ncpu = \"6502\"\nentry = \"start\"\n{inputs}\n{extra}")
}

#[test]
fn two_sources_link_to_the_same_bytes_as_one_whole_program() {
    let p = Project::new("two");
    p.file("main.asm", MAIN).file("lib.asm", LIB);
    let layout = layout_with(
        "[[input]]\nsrc = [\"main.asm\", \"lib.asm\"]\n\
         [[segment]]\nname = \"CODE\"\nstart = 0x0801\n",
        "",
    );
    let linked = p.build(&layout, "out.bin").unwrap();
    // the whole program lays out CODE (main then lib) then RODATA in this order too
    let whole = {
        std::fs::write(p.path().join("whole.asm"), WHOLE).unwrap();
        let l = layout_with("[[input]]\nasm = \"whole.asm\"\n", "");
        p.build(&l, "whole.bin").unwrap()
    };
    assert_eq!(linked.bytes, whole.bytes);
}

#[test]
fn a_library_member_is_linked_only_when_needed() {
    let p = Project::new("lib");
    p.file("main.asm", MAIN).file("lib.asm", LIB).file(
        "unused.asm",
        ".export never\nnever: .byte $EE,$EE,$EE,$EE\n",
    );
    let mut a = Archive::new();
    a.add("lib.rco", &p.object("lib.asm"));
    a.add("unused.rco", &p.object("unused.asm"));
    std::fs::write(p.path().join("lib.rca"), a.encode().unwrap()).unwrap();
    let layout = layout_with(
        "[[input]]\nsrc = \"main.asm\"\n[[input]]\nlib = \"lib.rca\"\n\
         [[segment]]\nname = \"CODE\"\nstart = 0x0801\n",
        "",
    );
    let b = p.build(&layout, "out.bin").unwrap();
    assert!(
        !b.bytes.windows(4).any(|w| w == [0xEE; 4]),
        "unused member linked"
    );
    assert!(b.map.contains("lib.rca(lib.rco)"), "{}", b.map);
    assert!(!b.map.contains("unused.rco"), "{}", b.map);
}

#[test]
fn a_member_that_needs_another_member_pulls_it_in() {
    let p = Project::new("chain");
    p.file(
        "main.asm",
        ".import fa
.export start
start: jsr fa
 rts
",
    )
    .file(
        "a.asm",
        ".import fb
.export fa
fa: jsr fb
 rts
",
    )
    .file(
        "b.asm",
        ".export fb
fb: inc $d020
 rts
",
    );
    let mut lib = Archive::new();
    // b first: order inside a library must not matter
    lib.add("b.rco", &p.object("b.asm"));
    lib.add("a.rco", &p.object("a.asm"));
    std::fs::write(p.path().join("l.rca"), lib.encode().unwrap()).unwrap();
    let layout = layout_with(
        "[[input]]\nsrc = \"main.asm\"\n[[input]]\nlib = \"l.rca\"\n[[segment]]\nname = \"CODE\"\nstart = 0x1000\n",
        "",
    );
    let b = p.build(&layout, "out.bin").unwrap();
    // start: jsr fa ($1004); fa: jsr fb
    assert_eq!(b.bytes[..3], [0x20, 0x04, 0x10]);
}

#[test]
fn an_undefined_import_names_the_symbol_and_who_wants_it() {
    let p = Project::new("undef");
    p.file(
        "main.asm",
        ".import missing\n.export start\nstart: jsr missing\n",
    );
    let layout = layout_with(
        "[[input]]\nsrc = \"main.asm\"\n[[segment]]\nname = \"CODE\"\nstart = 0x1000\n",
        "",
    );
    let e = p.build(&layout, "out.bin").unwrap_err();
    assert!(e.contains("undefined symbol \"missing\""), "{e}");
    assert!(e.contains("main.asm"), "{e}");
}

#[test]
fn two_definitions_of_a_global_name_both_files() {
    let p = Project::new("dup");
    p.file("a.asm", ".export f\nf: rts\n")
        .file("b.asm", ".export f\nf: rts\n");
    let layout = layout_with(
        "[[input]]\nsrc = [\"a.asm\", \"b.asm\"]\n[[segment]]\nname = \"CODE\"\nstart = 0x1000\n",
        "",
    );
    let e = p.build(&layout, "out.bin").unwrap_err();
    assert!(
        e.contains("defined in both") && e.contains("a.asm") && e.contains("b.asm"),
        "{e}"
    );
}

#[test]
fn a_branch_too_far_is_an_error_with_the_distance() {
    let p = Project::new("far");
    let mut far = String::from(".segment \"A\"\n.export start\nstart: bne target\n");
    far.push_str(".segment \"B\"\n");
    far.push_str(&" .byte 0\n".repeat(200));
    far.push_str(".export target\ntarget: rts\n");
    p.file("far.asm", &far);
    let layout = layout_with(
        "[[input]]\nsrc = \"far.asm\"\n[[segment]]\nname = \"A\"\nstart = 0x1000\n[[segment]]\nname = \"B\"\nstart = 0x1010\n",
        "",
    );
    let e = p.build(&layout, "out.bin").unwrap_err();
    assert!(e.contains("branch distance"), "{e}");
    assert!(e.contains("far.asm:"), "{e}");
}

#[test]
fn a_value_that_does_not_fit_names_the_symbol() {
    let p = Project::new("fit");
    p.file(
        "a.asm",
        ".export start\nstart: lda #<big\n ldx #big\n.segment \"D\"\nbig: .byte 1\n",
    );
    let layout = layout_with(
        "[[input]]\nsrc = \"a.asm\"\n[[segment]]\nname = \"CODE\"\nstart = 0x1000\n[[segment]]\nname = \"D\"\nstart = 0x2000\n",
        "",
    );
    let e = p.build(&layout, "out.bin").unwrap_err();
    assert!(e.contains("big") && e.contains("does not fit"), "{e}");
}

#[test]
fn a_segment_larger_than_its_region_is_an_error() {
    let p = Project::new("region");
    p.file(
        "a.asm",
        ".export start\nstart: nop\n.segment \"DATA\"\n.fill 300, 1\n",
    );
    let layout = layout_with(
        "[[input]]\nsrc = \"a.asm\"\n\
         [[region]]\nname = \"low\"\nstart = 0x1000\nend = 0x10FF\n\
         [[region]]\nname = \"high\"\nstart = 0x2000\nend = 0x20FF\n\
         [[segment]]\nname = \"CODE\"\nregion = \"low\"\n\
         [[segment]]\nname = \"DATA\"\nregion = \"high\"\n",
        "",
    );
    let e = p.build(&layout, "out.bin").unwrap_err();
    assert!(
        e.contains("\"DATA\"") && e.contains("does not fit in region \"high\""),
        "{e}"
    );
}

#[test]
fn a_segment_without_an_address_says_so() {
    let p = Project::new("noaddr");
    p.file("a.asm", ".export start\nstart: nop\n");
    let layout = layout_with("[[input]]\nsrc = \"a.asm\"\n", "");
    let e = p.build(&layout, "out.bin").unwrap_err();
    assert!(e.contains("\"CODE\"") && e.contains("no address"), "{e}");
}

#[test]
fn bss_takes_space_but_no_bytes_and_symbols_resolve() {
    let p = Project::new("bss");
    p.file(
        "a.asm",
        ".export start\nstart: lda counter\n sta counter+1\n rts\n.segment \"BSS\", bss\ncounter: .res 2\n",
    );
    let layout = layout_with(
        "[[input]]\nsrc = \"a.asm\"\n[[segment]]\nname = \"CODE\"\nstart = 0x1000\n[[segment]]\nname = \"BSS\"\nstart = 0x3000\n",
        "",
    );
    let b = p.build(&layout, "out.bin").unwrap();
    // lda $3000; sta $3001; rts
    assert_eq!(b.bytes, [0xAD, 0x00, 0x30, 0x8D, 0x01, 0x30, 0x60]);
}

#[test]
fn a_prebuilt_object_links_like_a_source() {
    let p = Project::new("obj");
    p.file("main.asm", MAIN).file("lib.asm", LIB);
    std::fs::write(p.path().join("main.rco"), p.object("main.asm").encode()).unwrap();
    std::fs::write(p.path().join("lib.rco"), p.object("lib.asm").encode()).unwrap();
    let a = layout_with(
        "[[input]]\nobj = [\"main.rco\", \"lib.rco\"]\n[[segment]]\nname = \"CODE\"\nstart = 0x0801\n",
        "",
    );
    let b = layout_with(
        "[[input]]\nsrc = [\"main.asm\", \"lib.asm\"]\n[[segment]]\nname = \"CODE\"\nstart = 0x0801\n",
        "",
    );
    assert_eq!(
        p.build(&a, "a.bin").unwrap().bytes,
        p.build(&b, "b.bin").unwrap().bytes
    );
}

#[test]
fn damaged_objects_are_errors_not_panics() {
    let p = Project::new("bad");
    p.file("a.asm", ".export start\nstart: rts\n");
    let good = p.object("a.asm").encode();
    for n in [0, 3, good.len() / 2, good.len() - 1] {
        std::fs::write(p.path().join("a.rco"), &good[..n]).unwrap();
        let layout = layout_with("[[input]]\nobj = \"a.rco\"\n", "");
        assert!(p.build(&layout, "out.bin").is_err());
    }
}
