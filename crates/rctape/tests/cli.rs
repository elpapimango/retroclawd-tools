//! `rctape` as a program.

use std::path::PathBuf;
use std::process::{Command, Output};

fn rctape(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rctape"))
        .args(args)
        .output()
        .expect("runs rctape")
}

fn ok(args: &[&str]) -> String {
    let out = rctape(args);
    assert!(
        out.status.success(),
        "rctape {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn fails(args: &[&str]) -> String {
    let out = rctape(args);
    assert!(!out.status.success(), "rctape {args:?} should fail");
    String::from_utf8_lossy(&out.stderr).into_owned() + &String::from_utf8_lossy(&out.stdout)
}

struct Temp(PathBuf);

impl Temp {
    fn new(tag: &str) -> Temp {
        let dir = std::env::temp_dir().join(format!("rctape-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Temp(dir)
    }
    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
    fn write(&self, name: &str, bytes: &[u8]) -> String {
        std::fs::write(self.0.join(name), bytes).unwrap();
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

fn bytes(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(7).wrapping_add(seed))
        .collect()
}

/// A tape with two programs, made the way a user would.
fn two_programs(t: &Temp) -> String {
    let a = t.write("alpha.bin", &bytes(300, 1));
    let b = t.write("beta.bin", &bytes(40, 2));
    let tape = t.path("two.tzx");
    ok(&["add", &tape, &a, "--load", "0x8000"]);
    ok(&["add", &tape, &b, "--load", "0xC000", "--name", "Second"]);
    tape
}

fn add_list_extract_round_trip() {
    let t = Temp::new("add");
    let tape = two_programs(&t);
    let list = ok(&["list", &tape]);
    assert!(
        list.contains("Bytes: alpha start 32768, length 300"),
        "{list}"
    );
    assert!(
        list.contains("Bytes: Second start 49152, length 40"),
        "{list}"
    );
    assert_eq!(list.lines().count(), 4);
    let out = t.path("out");
    ok(&["extract", &tape, "-d", &out]);
    assert_eq!(t.read("out/alpha.bin"), bytes(300, 1));
    assert_eq!(t.read("out/Second.bin"), bytes(40, 2));
    assert!(fails(&["extract", &tape, "-d", &out]).contains("--force"));
    ok(&["extract", &tape, "-d", &out, "--force"]);
    // One block to an exact path.
    let one = t.path("one.bin");
    ok(&["extract", &tape, "4", "-o", &one]);
    assert_eq!(t.read("one.bin"), bytes(40, 2));
    assert!(fails(&["extract", &tape, "-o", &t.path("many.bin")]).contains("exactly one"));
}

fn json_listing_and_info() {
    let t = Temp::new("json");
    let tape = two_programs(&t);
    let json = ok(&["list", &tape, "--json"]);
    assert!(
        json.contains("\"kind\": \"Standard speed data\"")
            && json.contains("\"checksum_ok\": true"),
        "{json}"
    );
    let info = ok(&["info", &tape]);
    assert!(
        info.contains("TZX 1.20") && info.contains("blocks:    4"),
        "{info}"
    );
    assert!(info.contains("all good"), "{info}");
    assert!(ok(&["info", &tape, "--json"]).contains("\"blocks\": 4"));
}

fn convert_between_tap_and_tzx_keeps_the_data() {
    let t = Temp::new("convert");
    let tape = two_programs(&t);
    let tap = t.path("two.tap");
    ok(&["convert", &tape, &tap]);
    let back = t.path("back.tzx");
    ok(&["convert", &tap, &back]);
    assert_eq!(ok(&["list", &tape]), ok(&["list", &back]));
    // Pauses are in the .tzx only: a tape with a pause block does not fit a .tap unless told to drop it.
    let mut with_pause = std::fs::read(&tape).unwrap();
    with_pause.extend_from_slice(&[0x20, 0xE8, 0x03]);
    let p = t.write("pause.tzx", &with_pause);
    assert!(fails(&["convert", &p, &t.path("p.tap")]).contains("--lossy"));
    ok(&["convert", &p, &t.path("p.tap"), "--lossy"]);
    assert_eq!(t.read("p.tap"), t.read("two.tap"));
    assert!(fails(&["convert", &tape, &t.path("x.wav")]).contains(".tap or .tzx"));
}

fn cut_rm_merge_split() {
    let t = Temp::new("edit");
    let tape = two_programs(&t);
    let second = t.path("second.tzx");
    ok(&["cut", &tape, "3-4", "-o", &second]);
    assert!(ok(&["list", &second]).contains("Second"));
    assert_eq!(ok(&["list", &second]).lines().count(), 2);
    // rm changes the tape in place; -o keeps the original.
    let copy = t.path("copy.tzx");
    ok(&["rm", &tape, "1-2", "-o", &copy]);
    assert_eq!(ok(&["list", &tape]).lines().count(), 4);
    assert_eq!(ok(&["list", &copy]).lines().count(), 2);
    ok(&["rm", &tape, "1-2"]);
    assert_eq!(ok(&["list", &tape]), ok(&["list", &second]));
    assert!(fails(&["rm", &tape, "9"]).contains("only 2"));
    // merge and split
    let merged = t.path("merged.tap");
    ok(&["merge", &second, &copy, "-o", &merged]);
    assert_eq!(ok(&["list", &merged]).lines().count(), 4);
    let parts = t.path("parts");
    ok(&["split", &merged, "-d", &parts]);
    let mut names: Vec<String> = std::fs::read_dir(&parts)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["01-Second.tap", "02-Second-2.tap"]);
    assert!(fails(&["merge", &second, "-o", &t.path("m.tzx")]).contains("at least two"));
}
/// Merged (every check kept): `convert_between_tap_and_tzx_keeps_the_data`, `cut_rm_merge_split`.
#[test]
fn convert_and_edit() {
    convert_between_tap_and_tzx_keeps_the_data();
    cut_rm_merge_split();
}

#[test]
fn validate_finds_a_wrong_checksum_and_cleanup_fixes_it() {
    let t = Temp::new("validate");
    let tape = two_programs(&t);
    assert_eq!(ok(&["validate", &tape]).trim(), "ok");
    let mut bytes = std::fs::read(&tape).unwrap();
    let n = bytes.len();
    bytes[n - 3] ^= 0x55; // inside the last data block
    let bad = t.write("bad.tzx", &bytes);
    let out = rctape(&["validate", &bad]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stdout).contains("block 4: wrong checksum"));
    assert!(ok(&["list", &bad]).contains("BAD CHECKSUM"));
    let fixed = t.path("fixed.tzx");
    ok(&["cleanup", &bad, "--fix-checksums", "-o", &fixed]);
    assert_eq!(ok(&["validate", &fixed]).trim(), "ok");
    // A header whose data is missing is a problem too.
    ok(&["rm", &fixed, "4"]);
    let out = rctape(&["validate", &fixed]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stdout).contains("has no data block"));
}

fn cat_prints_data_hex_and_basic() {
    let t = Temp::new("cat");
    let listing = t.write(
        "prog.bas",
        &[0, 10, 6, 0, 0xF5, b'"', b'H', b'I', b'"', 0x0D],
    );
    let tape = t.path("p.tap");
    ok(&["add", &tape, &listing, "--type", "basic", "--line", "10"]);
    assert!(ok(&["list", &tape]).contains("Program: prog LINE 10 (10 bytes)"));
    assert_eq!(ok(&["cat", &tape, "2", "--basic"]), "10 PRINT \"HI\"\n");
    assert!(ok(&["cat", &tape, "2", "--hex"]).starts_with("0000  00 0A 06 00 F5 22 48 49 22 0D"));
    let raw = rctape(&["cat", &tape, "2"]);
    assert_eq!(raw.stdout, std::fs::read(&listing).unwrap());
    assert!(fails(&["cat", &tape, "3"]).contains("no block 3"));
}
/// Merged (every check kept): `add_list_extract_round_trip`, `json_listing_and_info`, `cat_prints_data_hex_and_basic`.
#[test]
fn listing_commands() {
    add_list_extract_round_trip();
    json_listing_and_info();
    cat_prints_data_hex_and_basic();
}

fn a_headerless_data_block_and_clear_errors() {
    let t = Temp::new("data");
    let f = t.write("raw.dat", &[1, 2, 3]);
    let tape = t.path("d.tap");
    ok(&["add", &tape, &f, "--type", "data"]);
    assert_eq!(ok(&["list", &tape]).lines().count(), 1);
    assert!(fails(&["add", &tape, &f, "--type", "nope"]).contains("--type must be"));
    assert!(fails(&["list", &t.write("junk.tap", &[1])]).contains("junk.tap"));
    assert!(fails(&["list", &t.path("missing.tzx")]).contains("cannot read"));
}

/// A `.t64` with one program, built by hand the way the format lays it out.
fn t64(name: &[u8], load: u16, data: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; 64];
    v[..20].copy_from_slice(b"C64S tape file\0\0\0\0\0\0");
    v[32] = 0; // version 0x0100
    v[33] = 1;
    v[34] = 1; // entries
    v[36] = 1; // used
    let mut rec = vec![0u8; 32];
    rec[0] = 1;
    rec[1] = 0x82;
    rec[2..4].copy_from_slice(&load.to_le_bytes());
    rec[4..6].copy_from_slice(&(load + data.len() as u16).to_le_bytes());
    rec[8..12].copy_from_slice(&96u32.to_le_bytes());
    rec[16..32].fill(0x20);
    rec[16..16 + name.len()].copy_from_slice(name);
    v.extend(rec);
    v.extend_from_slice(data);
    v
}

fn a_t64_lists_and_extracts_prg_files() {
    let t = Temp::new("t64");
    let file = t.write("game.t64", &t64(b"HELLO", 0x0801, &[1, 2, 3, 4, 5]));
    let list = ok(&["list", &file]);
    assert!(
        list.contains("HELLO") && list.contains("$0801-$0806") && list.contains("PRG"),
        "{list}"
    );
    ok(&["extract", &file, "-d", &t.path("out")]);
    assert_eq!(t.read("out/HELLO.prg"), [0x01, 0x08, 1, 2, 3, 4, 5]);
    assert!(ok(&["info", &file]).contains("entries:   1"));
    assert!(fails(&["rm", &file, "1"]).contains("no blocks"));
}

fn a_commodore_tap_is_described() {
    let t = Temp::new("c64tap");
    let mut tap = b"C64-TAPE-RAW".to_vec();
    tap.extend_from_slice(&[1, 0, 0, 0, 3, 0, 0, 0]);
    tap.extend_from_slice(&[0x30, 0x42, 0x56]);
    let file = t.write("x.tap", &tap);
    let info = ok(&["info", &file]);
    assert!(
        info.contains("version 1") && info.contains("PAL") && info.contains("pulses:    3"),
        "{info}"
    );
    assert!(info.contains("384 to 688"), "{info}");
    assert!(fails(&["extract", &file]).contains("pulses"));
}
/// Merged (every check kept): `a_headerless_data_block_and_clear_errors`, `a_t64_lists_and_extracts_prg_files`, `a_commodore_tap_is_described`.
#[test]
fn tape_edge_cases() {
    a_headerless_data_block_and_clear_errors();
    a_t64_lists_and_extracts_prg_files();
    a_commodore_tap_is_described();
}
