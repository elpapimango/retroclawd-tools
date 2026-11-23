//! ld65 `.cfg` files drive the linker.

use rcld::cfg;
use rcld::layout;
use std::path::PathBuf;

const C64_CFG: &str = r#"
# a typical c64 config
FEATURES { STARTADDRESS: default = $0801; }
SYMBOLS  { __LOADADDR__: type = import; }
MEMORY {
    ZP:     file = "",               start = $0002, size = $001A;
    MAIN:   file = %O, define = yes, start = $0801, size = $C000 - $0801;
    DATA:   file = "", start = $C000, size = $0100;
}
SEGMENTS {
    ZEROPAGE: load = ZP,   type = zp;
    CODE:     load = MAIN, type = ro;
    RODATA:   load = MAIN, type = ro, align = $10;
    DATA:     load = MAIN, type = rw;
    BSS:      load = DATA, type = bss, define = yes;
}
"#;

#[test]
fn memory_and_segments_become_regions_and_rules() {
    let c = cfg::parse(C64_CFG).unwrap();
    let names: Vec<&str> = c.regions.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["ZP", "MAIN", "DATA"]);
    assert_eq!(c.regions[1].start, layout::Value::Int(0x0801));
    assert_eq!(c.regions[1].end, layout::Value::Int(0xBFFF));
    assert_eq!(c.regions[2].end, layout::Value::Int(0xC0FF));
    assert_eq!(c.segments.len(), 5);
    assert_eq!(c.segments[2].region.as_deref(), Some("MAIN"));
    assert_eq!(c.segments[2].align, Some(16));
    assert_eq!(c.segments[4].region.as_deref(), Some("DATA"));
}

#[test]
fn errors_are_named() {
    assert!(
        cfg::parse("MEMORY { A: start = $1000; }")
            .unwrap_err()
            .contains("no size")
    );
    assert!(
        cfg::parse("MEMORY { A: start = __X__, size = 4; }")
            .unwrap_err()
            .contains("not a number")
    );
    assert!(
        cfg::parse("MEMORY { A start = 1; }")
            .unwrap_err()
            .contains("expected :")
    );
    assert!(
        cfg::parse("MEMORY { A: start = 1, size = 2;")
            .unwrap_err()
            .contains("missing")
    );
}

#[test]
fn a_project_links_by_a_cfg_file() {
    let dir: PathBuf = std::env::temp_dir().join(format!("rcld-cfg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("main.asm"),
        ".import msg\n.export start\n.segment \"CODE\"\nstart: ldx #0\nloop: lda msg,x\n sta $0400,x\n inx\n bne loop\n rts\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("data.asm"),
        ".export msg\n.segment \"RODATA\"\nmsg: .byte 1,2,3\n.segment \"BSS\"\nbuf: .res 8\n",
    )
    .unwrap();
    std::fs::write(dir.join("c64.cfg"), C64_CFG).unwrap();
    let l = layout::parse(
        "machine = \"generic\"\ncpu = \"6502\"\nentry = \"start\"\ncfg = \"c64.cfg\"\ngaps = \"fill\"\n[[input]]\nsrc = [\"main.asm\", \"data.asm\"]\n",
    )
    .unwrap();
    let b = rcld::build(&l, &dir, &dir.join("out.bin"), &[]).unwrap();
    // CODE at $0801 (12 bytes), RODATA aligned to $10 -> $0810
    assert_eq!(b.bytes[0], 0xA2);
    assert!(
        b.symbols_text.contains("start = $0801"),
        "{}",
        b.symbols_text
    );
    assert!(b.symbols_text.contains("msg = $0810"), "{}", b.symbols_text);
    assert!(b.symbols_text.contains("buf = $C000"), "{}", b.symbols_text);
    // the lda reads $0810
    assert_eq!(&b.bytes[2..5], &[0xBD, 0x10, 0x08]);
    let _ = std::fs::remove_dir_all(&dir);
}
