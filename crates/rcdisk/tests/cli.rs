//! `rcdisk` as a program: make disks, put files on them, take them off again, convert.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn rcdisk(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rcdisk"))
        .args(args)
        .output()
        .expect("runs rcdisk")
}

fn ok(args: &[&str]) -> String {
    let out = rcdisk(args);
    assert!(
        out.status.success(),
        "rcdisk {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn fails(args: &[&str]) -> String {
    let out = rcdisk(args);
    assert!(!out.status.success(), "rcdisk {args:?} should fail");
    String::from_utf8_lossy(&out.stderr).into_owned() + &String::from_utf8_lossy(&out.stdout)
}

struct Temp(PathBuf);

impl Temp {
    fn new(tag: &str) -> Temp {
        let dir = std::env::temp_dir().join(format!("rcdisk-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Temp(dir)
    }
    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn bytes(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(13).wrapping_add(seed))
        .collect()
}

fn every_commodore_format_round_trips_files() {
    for (file, format) in [
        ("a.d64", "d64"),
        ("a.x64", "x64"),
        ("a.d71", "d71"),
        ("a.d81", "d81"),
        ("b.img", "d64-40"),
    ] {
        let t = Temp::new(&format!("rt-{format}"));
        let image = t.path(file);
        ok(&[
            "create",
            &image,
            "--format",
            format,
            "--name",
            "Round Trip",
            "--id",
            "rt",
        ]);
        let hello = t.path("hello.prg");
        let notes = t.path("notes.seq");
        std::fs::write(&hello, bytes(700, 1)).unwrap();
        std::fs::write(&notes, bytes(5000, 2)).unwrap();
        ok(&["add", &image, &hello, &notes]);

        let listing = ok(&["list", &image]);
        assert!(listing.contains("\"ROUND TRIP      \" RT"), "{listing}");
        assert!(
            listing.contains("\"HELLO\"") && listing.contains("PRG"),
            "{listing}"
        );
        assert!(
            listing.contains("\"NOTES\"") && listing.contains("SEQ"),
            "{listing}"
        );

        let json = ok(&["list", &image, "--json"]);
        assert!(
            json.contains("\"name\": \"HELLO\"") && json.contains("\"type\": \"SEQ\""),
            "{json}"
        );

        let out = t.path("out");
        ok(&["extract", &image, "-d", &out]);
        assert_eq!(
            std::fs::read(Path::new(&out).join("HELLO.prg")).unwrap(),
            bytes(700, 1)
        );
        assert_eq!(
            std::fs::read(Path::new(&out).join("NOTES.seq")).unwrap(),
            bytes(5000, 2)
        );
        let single = t.path("single.bin");
        ok(&["extract", &image, "he*", "-o", &single]);
        assert_eq!(std::fs::read(&single).unwrap(), bytes(700, 1));

        assert!(ok(&["validate", &image]).contains("0 error(s), 0 warning(s)"));
        ok(&["rename", &image, "HELLO", "WORLD"]);
        ok(&["rm", &image, "NOTES"]);
        let listing = ok(&["list", &image]);
        assert!(
            listing.contains("\"WORLD\"") && !listing.contains("NOTES"),
            "{listing}"
        );
        assert!(ok(&["validate", &image]).contains("0 error(s)"));
        assert!(ok(&["info", &image]).contains(format.split('-').next().unwrap()));
    }
}

#[test]
fn modifying_commands_edit_in_place_or_write_a_copy() {
    let t = Temp::new("inplace");
    let image = t.path("a.d64");
    ok(&["create", &image]);
    let f = t.path("x.prg");
    std::fs::write(&f, [1, 8, 0, 0]).unwrap();
    let copy = t.path("copy.d64");
    ok(&["add", &image, &f, "-o", &copy]);
    assert!(!ok(&["list", &image]).contains("\"X\""));
    assert!(ok(&["list", &copy]).contains("\"X\""));
    // The copy is not overwritten a second time without --force; in-place editing is the command's purpose.
    assert!(fails(&["add", &image, &f, "-o", &copy]).contains("--force"));
    ok(&["add", &image, &f, "-o", &copy, "--force", "--replace"]);
    ok(&["add", &image, &f]);
    assert!(ok(&["list", &image]).contains("\"X\""));
}

fn errors_are_reported_and_leave_the_image_alone() {
    let t = Temp::new("errors");
    let image = t.path("a.d64");
    ok(&["create", &image]);
    let before = std::fs::read(&image).unwrap();
    let f = t.path("a long file name that does not fit.prg");
    std::fs::write(&f, [1, 8]).unwrap();
    assert!(fails(&["add", &image, &f]).contains("--name"));
    assert!(fails(&["rm", &image, "NOPE"]).contains("no file matching"));
    assert!(fails(&["rename", &image, "NOPE", "X"]).contains("no file matching"));
    assert!(fails(&["list", &t.path("missing.d64")]).contains("cannot read"));
    assert!(fails(&["create", &image]).contains("--force"));
    assert!(fails(&["create", &t.path("x.zzz")]).contains("--format"));
    ok(&["add", &image, &f, "--name", "short"]);
    assert!(fails(&["add", &image, &f, "--name", "short"]).contains("exists"));
    ok(&["add", &image, &f, "--name", "short", "--replace"]);
    assert_ne!(std::fs::read(&image).unwrap(), before);
    let junk = t.path("junk.d64");
    std::fs::write(&junk, b"not a disk").unwrap();
    assert!(fails(&["list", &junk]).contains("not a recognized"));
}

fn validate_finds_damage_and_exits_with_an_error() {
    let t = Temp::new("damage");
    let image = t.path("a.d64");
    ok(&["create", &image]);
    let f = t.path("x.prg");
    std::fs::write(&f, bytes(600, 4)).unwrap();
    ok(&["add", &image, &f]);
    // Free the BAM bit of the directory's first sector (track 18 sector 1): the file system now has a
    // sector in use that the BAM calls free.
    let mut raw = std::fs::read(&image).unwrap();
    let bam = 17 * 21 * 256; // track 18 starts after 17 tracks of 21 sectors
    raw[bam + 4 + 17 * 4 + 1] |= 0x02; // track 18's bitmap byte 0, sector 1
    std::fs::write(&image, raw).unwrap();
    let out = rcdisk(&["validate", &image]);
    assert!(!out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("lists it as free"), "{text}");
    let json = ok_json_allow_fail(&["validate", &image, "--json"]);
    assert!(json.contains("\"ok\": false"), "{json}");
}

fn ok_json_allow_fail(args: &[&str]) -> String {
    String::from_utf8_lossy(&rcdisk(args).stdout).into_owned()
}

fn conversion_keeps_files_and_the_disk_label() {
    let t = Temp::new("convert");
    let d64 = t.path("a.d64");
    ok(&["create", &d64, "--name", "MOVING", "--id", "mv"]);
    let f = t.path("big.prg");
    std::fs::write(&f, bytes(30000, 5)).unwrap();
    let g = t.path("tiny.seq");
    std::fs::write(&g, bytes(3, 6)).unwrap();
    ok(&["add", &d64, &f, &g]);
    for target in ["b.d81", "b.d71", "b.x64", "b.d64"] {
        let out = t.path(target);
        ok(&["convert", &d64, &out]);
        let listing = ok(&["list", &out]);
        assert!(
            listing.contains("\"MOVING          \" MV"),
            "{target}: {listing}"
        );
        let x = t.path(&format!("x-{target}"));
        ok(&["extract", &out, "-d", &x]);
        assert_eq!(
            std::fs::read(Path::new(&x).join("BIG.prg")).unwrap(),
            bytes(30000, 5)
        );
        assert_eq!(
            std::fs::read(Path::new(&x).join("TINY.seq")).unwrap(),
            bytes(3, 6)
        );
        assert!(ok(&["validate", &out]).contains("0 error(s)"), "{target}");
    }
    // d64 <-> x64 changes only the container.
    let x64 = t.path("b.x64");
    let back = t.path("back.d64");
    ok(&["convert", &x64, &back]);
    assert_eq!(std::fs::read(&back).unwrap(), std::fs::read(&d64).unwrap());
    let x64_bytes = std::fs::read(&x64).unwrap();
    assert_eq!(&x64_bytes[..4], &[0x43, 0x15, 0x41, 0x64]);
    assert_eq!(x64_bytes.len(), 64 + std::fs::read(&d64).unwrap().len());
    // Files that cannot be carried over are reported, not silently dropped (a .d81 holds more than a .d64).
    let big81 = t.path("big.d81");
    ok(&["create", &big81]);
    let huge = t.path("huge.prg");
    std::fs::write(&huge, bytes(400_000, 1)).unwrap();
    ok(&["add", &big81, &huge]);
    let out = rcdisk(&["convert", &big81, &t.path("small.d64")]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("disk full"));
}

fn cpm_disks_round_trip_files_with_and_without_headers() {
    for format in ["dsk", "dsk-system", "dsk-ibm", "dsk-plus3"] {
        let t = Temp::new(&format!("cpm-{format}"));
        let image = t.path("a.dsk");
        ok(&["create", &image, "--format", format]);
        let code = t.path("game.bin");
        let text = t.path("readme.txt");
        std::fs::write(&code, bytes(5000, 7)).unwrap();
        std::fs::write(&text, b"Hello from CP/M\r\n").unwrap();
        let header = if format == "dsk-plus3" {
            "plus3dos"
        } else {
            "amsdos"
        };
        ok(&[
            "add", &image, &code, "--header", header, "--load", "0x4000", "--entry", "0x4010",
        ]);
        ok(&["add", &image, &text]);
        ok(&["add", &image, &code, "--name", "other.bin", "--user", "3"]);

        let listing = ok(&["list", &image]);
        assert!(
            listing.contains("GAME.BIN") && listing.contains("5000"),
            "{format}: {listing}"
        );
        assert!(listing.contains("README.TXT"), "{listing}");
        assert!(listing.contains("3:OTHER.BIN"), "{listing}");
        if header == "amsdos" {
            assert!(
                listing.contains("BIN <load=&4000,entry=&4010>"),
                "{listing}"
            );
        } else {
            assert!(listing.contains("P3-COD <load=0x4000>"), "{listing}");
        }
        let json = ok(&["list", &image, "--json"]);
        assert!(
            json.contains("\"name\": \"GAME.BIN\"") && json.contains("\"size\": 5000"),
            "{json}"
        );

        // Extraction strips the header, so the original bytes come back exactly.
        let out = t.path("out");
        ok(&["extract", &image, "-d", &out]);
        assert_eq!(
            std::fs::read(Path::new(&out).join("GAME.BIN")).unwrap(),
            bytes(5000, 7)
        );
        // A file without a header comes back as whole 128-byte records.
        let readme = std::fs::read(Path::new(&out).join("README.TXT")).unwrap();
        assert_eq!(readme.len(), 128);
        assert_eq!(&readme[..17], b"Hello from CP/M\r\n");
        // --keep-header keeps it (and it is a valid header: rcdisk recognises it again on re-adding).
        let with = t.path("with.bin");
        ok(&["extract", &image, "GAME.BIN", "--keep-header", "-o", &with]);
        // Raw file contents: header, data, and CP/M's padding to whole 128-byte records.
        let raw = std::fs::read(&with).unwrap();
        assert_eq!(raw.len(), (5000_usize + 128).div_ceil(128) * 128);
        assert_eq!(&raw[128..128 + 5000], &bytes(5000, 7)[..]);

        assert!(
            ok(&["validate", &image]).contains("0 error(s), 0 warning(s)"),
            "{format}"
        );
        ok(&["rename", &image, "readme.txt", "notes.txt"]);
        ok(&["rm", &image, "3:*"]);
        let listing = ok(&["list", &image]);
        assert!(
            listing.contains("NOTES.TXT")
                && !listing.contains("README")
                && !listing.contains("OTHER"),
            "{listing}"
        );
        assert!(ok(&["validate", &image]).contains("0 error(s)"));
        assert!(ok(&["info", &image]).contains("CP/M"), "{format}");
    }
}

fn cpm_errors_and_limits() {
    let t = Temp::new("cpm-errors");
    let image = t.path("a.dsk");
    ok(&["create", &image]);
    let f = t.path("x.bin");
    std::fs::write(&f, [1, 2, 3]).unwrap();
    assert!(fails(&["add", &image, &f, "--header", "amsdos"]).contains("--load"));
    assert!(fails(&["add", &image, &f, "--name", "waytoolongname.bin"]).contains("8.3"));
    assert!(fails(&["add", &image, &f, "--header", "nonsense"]).contains("unknown header"));
    ok(&["add", &image, &f]);
    assert!(fails(&["add", &image, &f]).contains("exists"));
    ok(&["add", &image, &f, "--replace"]);
    // Too big for the disk: refused and the image is not changed.
    let before = std::fs::read(&image).unwrap();
    let big = t.path("big.bin");
    std::fs::write(&big, bytes(300_000, 1)).unwrap();
    assert!(fails(&["add", &image, &big]).contains("disk full"));
    assert_eq!(std::fs::read(&image).unwrap(), before);
    // A disk that is not CP/M is refused with a reason.
    let junk = t.path("junk.dsk");
    std::fs::write(&junk, b"MV - CPCEMU Disk-File\r\nDisk-Info\r\n").unwrap();
    assert!(!fails(&["list", &junk]).is_empty());
}
/// Merged (every check kept): `errors_are_reported_and_leave_the_image_alone`, `validate_finds_damage_and_exits_with_an_error`, `cpm_errors_and_limits`.
#[test]
fn errors_and_validation() {
    errors_are_reported_and_leave_the_image_alone();
    validate_finds_damage_and_exits_with_an_error();
    cpm_errors_and_limits();
}

fn conversion_between_cpm_and_commodore_goes_file_by_file() {
    let t = Temp::new("cpm-convert");
    let dsk = t.path("a.dsk");
    ok(&["create", &dsk]);
    let f = t.path("tune.bin");
    std::fs::write(&f, bytes(2000, 9)).unwrap();
    ok(&["add", &dsk, &f, "--header", "amsdos", "--load", "0x8000"]);
    let d64 = t.path("b.d64");
    ok(&["convert", &dsk, &d64]);
    let out = t.path("x");
    ok(&["extract", &d64, "-d", &out]);
    let entry = std::fs::read_dir(&out)
        .unwrap()
        .flatten()
        .next()
        .unwrap()
        .path();
    assert_eq!(std::fs::read(entry).unwrap(), bytes(2000, 9)); // the header is not carried over, the data is
    // And back: Commodore names are made to fit 8.3, and the change is reported.
    let c = t.path("long name.prg");
    std::fs::write(&c, bytes(300, 4)).unwrap();
    ok(&["add", &d64, &c, "--name", "A LONG FILE NAME"]);
    let back = t.path("c.dsk");
    let out = rcdisk(&["convert", &d64, &back]);
    assert!(out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("was stored as"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(ok(&["list", &back]).contains("A_LONG_F"));
    assert!(ok(&["validate", &back]).contains("0 error(s)"));
    // Between two CP/M layouts the files are kept.
    let plus3 = t.path("p.dsk");
    ok(&["convert", &dsk, &plus3, "--to", "dsk-plus3"]);
    assert!(ok(&["info", &plus3]).contains("Spectrum +3"));
}
/// Merged (every check kept): `every_commodore_format_round_trips_files`, `cpm_disks_round_trip_files_with_and_without_headers`, `conversion_keeps_files_and_the_disk_label`, `conversion_between_cpm_and_commodore_goes_file_by_file`.
#[test]
fn round_trips_and_conversions() {
    every_commodore_format_round_trips_files();
    cpm_disks_round_trip_files_with_and_without_headers();
    conversion_keeps_files_and_the_disk_label();
    conversion_between_cpm_and_commodore_goes_file_by_file();
}

// ---- real disks, when the machine repositories are checked out ----

#[test]
fn real_disks_validate_and_survive_an_extract_and_add_round_trip() {
    let Some(base) = std::env::var_os("RC_ROMS") else {
        eprintln!("skipping: RC_ROMS not set");
        return;
    };
    let dir = Path::new(&base).join("commodore/software/c64");
    let mut tested = 0;
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path
            .extension()
            .is_none_or(|e| !e.eq_ignore_ascii_case("d64"))
        {
            continue;
        }
        let p = path.to_string_lossy().into_owned();
        assert!(ok(&["validate", &p]).contains("0 error(s)"), "{p}");
        let t = Temp::new("real");
        let out = t.path("files");
        ok(&["extract", &p, "-d", &out]);
        // Every extracted file goes back onto a blank disk and comes out unchanged.
        let blank = t.path("blank.d64");
        ok(&["create", &blank]);
        let mut files: Vec<_> = std::fs::read_dir(&out)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        files.sort();
        for f in &files {
            // Names with characters a host file name cannot hold were changed by extract; give them their
            // own short disk names so the round trip is about the bytes.
            let stem = f
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .chars()
                .take(16)
                .collect::<String>();
            let kind = f.extension().unwrap().to_string_lossy().into_owned();
            ok(&[
                "add",
                &blank,
                &f.to_string_lossy(),
                "--name",
                &stem,
                "--type",
                &kind,
                "--replace",
            ]);
        }
        let again = t.path("again");
        ok(&["extract", &blank, "-d", &again]);
        for f in &files {
            let name = f.file_name().unwrap();
            let original = std::fs::read(f).unwrap();
            let copy = std::fs::read(Path::new(&again).join(name));
            assert_eq!(
                copy.ok().as_deref(),
                Some(original.as_slice()),
                "{p}: {name:?}"
            );
        }
        tested += 1;
    }
    if tested == 0 {
        eprintln!("skipping: no .d64 in {}", dir.display());
    }
}

/// An Atari ST disk from the command line: create, add, list, extract, and an executable boot sector.
#[test]
fn an_atari_st_disk_from_the_command_line() {
    let t = Temp::new("st");
    let disk = t.path("game.st");
    ok(&["create", &disk]);
    let file = t.path("hello.txt");
    std::fs::write(&file, b"Hello, ST\r\n").unwrap();
    ok(&["add", &disk, &file]);
    let listing = ok(&["list", &disk]);
    assert!(listing.contains("HELLO.TXT"), "{listing}");
    let code = t.path("boot.bin");
    std::fs::write(&code, [0x60, 0xFE]).unwrap();
    ok(&["boot", &disk, &code]);
    let b = std::fs::read(&disk).unwrap();
    let sum = b[..512].chunks(2).fold(0u16, |s, w| {
        s.wrapping_add(u16::from_be_bytes([w[0], w[1]]))
    });
    assert_eq!(sum, 0x1234);
    let out = t.path("back.txt");
    ok(&["extract", &disk, "HELLO.TXT", "-o", &out]);
    assert_eq!(std::fs::read(&out).unwrap(), b"Hello, ST\r\n");
    assert!(fails(&["boot", &file, &code]).contains("rcdisk"));
}
