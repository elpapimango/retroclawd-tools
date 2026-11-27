//! The Neo Geo target: a stored zip whose names carry the regions, read back with a small zip reader.

use rcld::layout;
use std::path::PathBuf;

struct Project(PathBuf);

impl Project {
    fn new(tag: &str) -> Project {
        let dir = std::env::temp_dir().join(format!("rcld-neo-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Project(dir)
    }
    fn file(&self, name: &str, bytes: &[u8]) -> &Project {
        std::fs::write(self.0.join(name), bytes).unwrap();
        self
    }
    fn build(&self, layout_text: &str) -> Result<rcld::Build, String> {
        let layout = layout::parse(layout_text)?;
        rcld::build(&layout, &self.0, &self.0.join("game.zip"), &[])
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn le32(b: &[u8], at: usize) -> usize {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as usize
}

/// (name, data) of every entry of a stored zip, checking each CRC.
fn read_zip(z: &[u8]) -> Vec<(String, Vec<u8>)> {
    let eocd = z.len() - 22;
    assert_eq!(le32(z, eocd), 0x0605_4B50);
    let count = u16::from_le_bytes([z[eocd + 10], z[eocd + 11]]) as usize;
    let mut at = le32(z, eocd + 16);
    let mut out = Vec::new();
    for _ in 0..count {
        assert_eq!(le32(z, at), 0x0201_4B50);
        let crc = le32(z, at + 16);
        let size = le32(z, at + 24);
        let nlen = u16::from_le_bytes([z[at + 28], z[at + 29]]) as usize;
        let local = le32(z, at + 42);
        let name = String::from_utf8(z[at + 46..at + 46 + nlen].to_vec()).unwrap();
        assert_eq!(le32(z, local), 0x0403_4B50);
        let lname = u16::from_le_bytes([z[local + 26], z[local + 27]]) as usize;
        let start = local + 30 + lname;
        let data = z[start..start + size].to_vec();
        // CRC-32 check
        let mut c = !0u32;
        for &b in &data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
        }
        assert_eq!(!c as usize, crc, "{name}");
        out.push((name, data));
        at += 46 + nlen;
    }
    out
}

const MAIN: &str = "        .org 0\n        dc.l $10F300, start\n        ds.b $F8\n        dc.b \"NEO-GEO\"\n        ds.b $F9\nstart:  bra.s start\n";

#[test]
fn a_set_has_the_regions_the_emulator_reads() {
    let p = Project::new("set");
    p.file("main.s", MAIN.as_bytes())
        .file("fix.bin", &[1, 2, 3, 4])
        .file("spr.bin", &[10, 20, 11, 21, 12, 22])
        .file("z80.bin", &[0xC9])
        .file("a.bin", &[7; 16])
        .file("b.bin", &[8; 16]);
    let b = p
        .build(
            "machine = \"neogeo\"\n[[input]]\nasm = \"main.s\"\n[options]\nngh = 42\ns = \"fix.bin\"\nc = \"spr.bin\"\nm = \"z80.bin\"\nv1 = \"a.bin\"\nv2 = \"b.bin\"\n",
        )
        .unwrap();
    let files = read_zip(&b.bytes);
    let names: Vec<&str> = files.iter().map(|f| f.0.as_str()).collect();
    assert_eq!(
        names,
        [
            "042-p1.p1",
            "042-s1.s1",
            "042-c1.c1",
            "042-c2.c2",
            "042-m1.m1",
            "042-v1.v11",
            "042-v2.v21"
        ]
    );
    assert_eq!(
        &files[0].1[0x100..0x106],
        b"EN-OEG",
        "program words are byte-swapped, as in the dumps"
    );
    assert_eq!(files[2].1, [10, 11, 12]);
    assert_eq!(files[3].1, [20, 21, 22]);
    assert_eq!(b.notes.len(), 1, "{:?}", b.notes);
    assert!(b.notes[0].contains("security"), "{:?}", b.notes);
}

/// A program whose header points `$182` at `$200`, where it reserves 122 bytes.
const WITH_POINTER: &str = "        .org 0
        dc.l $10F300, start
        ds.b $F8
        dc.b \"NEO-GEO\",0
        ds.b $7A
        dc.l code
        ds.b $200-$186
code:   ds.b 122
start:  bra.s start
";

#[test]
fn the_security_code_comes_from_a_file_or_a_cartridge_program() {
    let code: Vec<u8> = (0..122u8).map(|i| i.wrapping_mul(3)).collect();
    // a cartridge program file of its own, words swapped, its code at $400
    let mut cart = vec![0u8; 0x500];
    cart[0x100..0x107].copy_from_slice(b"NEO-GEO");
    cart[0x182..0x186].copy_from_slice(&0x400u32.to_be_bytes());
    cart[0x400..0x400 + 122].copy_from_slice(&code);
    for pair in cart.as_chunks_mut::<2>().0 {
        pair.swap(0, 1);
    }
    let p = Project::new("sec");
    p.file("main.s", WITH_POINTER.as_bytes())
        .file("code.bin", &code)
        .file("cart.p1", &cart)
        .file("short.bin", &[1, 2, 3]);
    let layout = |file: &str| {
        format!(
            "machine = \"neogeo\"
[[input]]
asm = \"main.s\"
[options]
security = \"{file}\"
"
        )
    };
    for file in ["code.bin", "cart.p1"] {
        let b = p.build(&layout(file)).unwrap();
        assert!(b.notes.is_empty(), "{file}: {:?}", b.notes);
        let mut prom = read_zip(&b.bytes).remove(0).1;
        for pair in prom.as_chunks_mut::<2>().0 {
            pair.swap(0, 1);
        }
        assert_eq!(&prom[0x200..0x200 + 122], code.as_slice(), "{file}");
    }
    let e = p.build(&layout("short.bin")).unwrap_err();
    assert!(e.contains("neither"), "{e}");
    let odd = WITH_POINTER.replace("dc.l code", "dc.l code+1");
    p.file("main.s", odd.as_bytes());
    let e = p.build(&layout("code.bin")).unwrap_err();
    assert!(e.contains("odd"), "{e}");
}

#[test]
fn mistakes_and_notes() {
    let p = Project::new("bad");
    p.file("plain.s", b"        .org 0\n        dc.l 0, 0\n");
    let b = p
        .build("machine = \"neogeo\"\n[[input]]\nasm = \"plain.s\"\n")
        .unwrap();
    assert!(b.notes.iter().any(|n| n.contains("NEO-GEO")));
    let e = p
        .build("machine = \"neogeo\"\n[[input]]\nasm = \"plain.s\"\n[options]\nngh = 99999\n")
        .unwrap_err();
    assert!(e.contains("four digits"), "{e}");
    let e = p
        .build(
            "machine = \"neogeo\"\n[[input]]\nasm = \"plain.s\"\n[options]\ns = \"missing.bin\"\n",
        )
        .unwrap_err();
    assert!(e.contains("options.s"), "{e}");
}
