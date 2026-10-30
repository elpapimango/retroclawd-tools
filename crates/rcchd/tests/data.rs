//! Hard disks, DVDs and raw data through `rcchd`: creation with the options chdman has, extraction, copies, delta files and metadata
//! edits. With `RC_CHDMAN` set (chdman as a black box) every file must also verify there and carry the hashes and metadata chdman
//! makes from the same input.

use chd::{Codec, Image};
use rcchd::hdgeom::Chs;
use rcchd::rawio::{self, Copy, CreateData, ExtractData, Media};
use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rcchd-data-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// Bytes with structure: text, runs, ramps and noise, so every codec has something to find.
fn sample(len: usize, seed0: u32) -> Vec<u8> {
    let mut seed = seed0;
    (0..len)
        .map(|i| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            match (i / 700) % 5 {
                0 => b"retroclawd makes chd files "[i % 27],
                1 => 0,
                2 => (i % 251) as u8,
                3 => ((i as f64 / 9.0).sin() * 90.0) as i8 as u8,
                _ => (seed >> 24) as u8 & 0x1F,
            }
        })
        .collect()
}

fn write(name: &str, bytes: &[u8]) -> PathBuf {
    let p = scratch(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

fn make(media: Media, input: &Path, out: &str, f: impl FnOnce(&mut CreateData)) -> PathBuf {
    let out = scratch(out);
    let mut o = CreateData::new(media, out.clone());
    o.input = Some(input.to_path_buf());
    o.force = true;
    f(&mut o);
    rawio::create(&o, |_, _| {}).unwrap();
    out
}

fn extract(media: Media, chd: &Path, parent: Option<&Path>, out: &str) -> Vec<u8> {
    let out = scratch(out);
    let o = ExtractData {
        media,
        input: chd.to_path_buf(),
        parent: parent.map(Path::to_path_buf),
        output: out.clone(),
        start_byte: None,
        start_hunk: None,
        bytes: None,
        hunks: None,
        force: true,
    };
    rawio::extract(&o, |_, _| {}).unwrap();
    std::fs::read(out).unwrap()
}

#[test]
fn a_hard_disk_image_round_trips_with_its_geometry() {
    let data = sample(512 * 6144, 1);
    let input = write("hd.img", &data);
    for codecs in [None, Some(vec![]), Some(vec![Codec::Zstd, Codec::Flac])] {
        let chd = make(Media::Hd, &input, "hd.chd", |o| o.codecs = codecs.clone());
        let mut image = Image::open(&chd, None).unwrap();
        assert!(image.verify(|_, _| {}).unwrap().ok());
        assert_eq!(
            (image.header().hunk_bytes, image.header().unit_bytes),
            (4096, 512)
        );
        assert_eq!(image.metadata()[0].tag, *b"GDDD");
        // 6144 sectors: 48 to a track, 16 heads, 8 cylinders
        let (chs, bps) = Chs::parse(&image.metadata()[0].data).unwrap();
        assert_eq!(
            (chs, bps),
            (
                Chs {
                    cylinders: 8,
                    heads: 16,
                    sectors: 48
                },
                512
            )
        );
        assert!(
            extract(Media::Hd, &chd, None, "hd.out") == data,
            "{codecs:?}"
        );
    }
}

#[test]
fn a_blank_disk_comes_from_a_size_a_geometry_or_a_template() {
    for (which, bytes) in [(0, 512 * 2000u64), (1, 512 * 6800), (2, 512 * 321 * 2 * 16)] {
        let out = scratch("blank.chd");
        let mut o = CreateData::new(Media::Hd, out.clone());
        o.force = true;
        match which {
            0 => o.size = Some(512 * 2000),
            1 => {
                o.chs = Some(Chs {
                    cylinders: 100,
                    heads: 4,
                    sectors: 17,
                })
            }
            _ => o.template = Some(1),
        }
        rawio::create(&o, |_, _| {}).unwrap();
        let mut image = Image::open(&out, None).unwrap();
        assert_eq!(image.header().logical_bytes, bytes);
        assert!(image.read_bytes(0, 4096).unwrap().iter().all(|&b| b == 0));
        assert!(
            !image.header().compressed(),
            "a blank disk is not compressed"
        );
    }
    // a geometry that does not match a given image, and no source of size at all, are refused
    let input = write("small.img", &sample(512 * 100, 2));
    let mut o = CreateData::new(Media::Hd, scratch("bad.chd"));
    o.input = Some(input);
    o.chs = Some(Chs {
        cylinders: 3,
        heads: 3,
        sectors: 3,
    });
    o.force = true;
    assert!(
        rawio::create(&o, |_, _| {})
            .unwrap_err()
            .0
            .contains("geometry")
    );
    let mut none = CreateData::new(Media::Hd, scratch("bad2.chd"));
    none.force = true;
    assert!(rawio::create(&none, |_, _| {}).is_err());
}

#[test]
fn raw_and_dvd_images_keep_their_units_and_cut_slices() {
    let data = sample(2048 * 300, 3);
    let input = write("d.iso", &data);
    let dvd = make(Media::Dvd, &input, "d.chd", |_| {});
    let image = Image::open(&dvd, None).unwrap();
    assert_eq!(
        (image.header().hunk_bytes, image.header().unit_bytes),
        (4096, 2048)
    );
    assert_eq!(image.metadata()[0].tag, *b"DVD ");
    assert!(extract(Media::Dvd, &dvd, None, "d.out") == data);
    // a DVD is not a hard disk
    let o = ExtractData {
        media: Media::Hd,
        input: dvd.clone(),
        parent: None,
        output: scratch("nope.img"),
        start_byte: None,
        start_hunk: None,
        bytes: None,
        hunks: None,
        force: true,
    };
    assert!(rawio::extract(&o, |_, _| {}).is_err());

    // raw: a unit size is required, the data must be whole units, a slice may start and end anywhere in units
    let raw = write("r.dat", &data[..100_000]);
    let mut none = CreateData::new(Media::Raw, scratch("r.chd"));
    none.input = Some(raw.clone());
    none.force = true;
    assert!(
        rawio::create(&none, |_, _| {})
            .unwrap_err()
            .0
            .contains("unitsize")
    );
    let sliced = make(Media::Raw, &raw, "r.chd", |o| {
        o.unit_bytes = Some(4);
        o.start_byte = Some(8192);
        o.bytes = Some(40_000);
    });
    assert!(extract(Media::Raw, &sliced, None, "r.out") == data[8192..8192 + 40_000]);
    let by_hunks = make(Media::Raw, &raw, "rh.chd", |o| {
        o.unit_bytes = Some(4);
        o.start_hunk = Some(2);
        o.hunks = Some(3);
    });
    assert!(extract(Media::Raw, &by_hunks, None, "rh.out") == data[8192..8192 + 3 * 4096]);
    let mut odd = CreateData::new(Media::Raw, scratch("odd.chd"));
    odd.input = Some(raw);
    odd.unit_bytes = Some(7);
    odd.force = true;
    assert!(
        rawio::create(&odd, |_, _| {})
            .unwrap_err()
            .0
            .contains("not divisible")
    );
}

#[test]
fn a_delta_file_is_small_and_needs_its_parent() {
    let base = sample(4096 * 40, 4);
    let mut changed = base.clone();
    changed[4096 * 7 + 3] ^= 0xFF;
    changed[4096 * 30] ^= 0x01;
    let base_path = write("base.dat", &base);
    let child_path = write("child.dat", &changed);
    let parent = make(Media::Raw, &base_path, "parent.chd", |o| {
        o.unit_bytes = Some(4)
    });
    let child = make(Media::Raw, &child_path, "child.chd", |o| {
        o.parent = Some(parent.clone())
    });
    assert!(
        std::fs::metadata(&child).unwrap().len() < 2500,
        "only two hunks differ"
    );
    assert!(Image::open(&child, None).is_err());
    assert!(extract(Media::Raw, &child, Some(&parent), "child.out") == changed);
    // copied standalone, the delta needs nobody
    let flat = scratch("flat.chd");
    let c = Copy {
        input: child,
        input_parent: Some(parent),
        output: flat.clone(),
        output_parent: None,
        codecs: None,
        hunk_bytes: None,
        force: true,
        threads: 0,
    };
    rawio::copy(&c, |_, _| {}).unwrap();
    assert!(extract(Media::Raw, &flat, None, "flat.out") == changed);
}

#[test]
fn copy_changes_codecs_and_hunk_size_and_keeps_data_and_metadata() {
    let data = sample(512 * 4096, 5);
    let input = write("c.img", &data);
    let src = make(Media::Hd, &input, "c.chd", |_| {});
    for (codecs, hunk) in [
        (Some(vec![Codec::Zlib]), None),
        (Some(vec![]), Some(8192)),
        (None, Some(2048)),
    ] {
        let out = scratch("c2.chd");
        let c = Copy {
            input: src.clone(),
            input_parent: None,
            output: out.clone(),
            output_parent: None,
            codecs,
            hunk_bytes: hunk,
            force: true,
            threads: 0,
        };
        rawio::copy(&c, |_, _| {}).unwrap();
        let a = Image::open(&src, None).unwrap();
        let mut b = Image::open(&out, None).unwrap();
        assert!(a.header().raw_sha1 == b.header().raw_sha1 || !b.header().compressed());
        assert_eq!(a.metadata()[0].data, b.metadata()[0].data);
        assert!(b.verify(|_, _| {}).unwrap().ok());
        assert!(extract(Media::Hd, &out, None, "c2.out") == data);
        let _ = a.hunk_count();
    }
}

#[test]
fn metadata_edits_dump_and_keep_the_file_valid() {
    let input = write("m.img", &sample(512 * 1024, 6));
    let chd = make(Media::Hd, &input, "m.chd", |_| {});
    chd::set_metadata(&chd, rawio::parse_tag("TEST").unwrap(), 0, b"hello\0", true).unwrap();
    chd::set_metadata(
        &chd,
        rawio::parse_tag("54455354").unwrap(),
        1,
        b"again\0",
        false,
    )
    .unwrap();
    assert_eq!(rawio::dump_metadata(&chd, *b"TEST", 1).unwrap(), b"again\0");
    chd::delete_metadata(&chd, *b"TEST", 0).unwrap();
    assert_eq!(rawio::dump_metadata(&chd, *b"TEST", 0).unwrap(), b"again\0");
    assert!(rawio::dump_metadata(&chd, *b"TEST", 1).is_err());
    assert!(
        Image::open(&chd, None)
            .unwrap()
            .verify(|_, _| {})
            .unwrap()
            .ok()
    );
    assert!(rawio::parse_tag("TOOLONG").is_err());
}

#[test]
#[ignore]
fn chdman_verifies_our_files_and_makes_the_same_hashes_from_the_same_options() {
    let Ok(tool) = std::env::var("RC_CHDMAN") else {
        return;
    };
    let hd = write("x-hd.img", &sample(512 * 16384, 7));
    let dvd = write("x-dvd.iso", &sample(2048 * 3000, 8));
    let raw = write("x-raw.dat", &sample(100_000, 9));
    let run = |args: &[&str]| -> String {
        let out = Command::new(&tool).args(args).output().unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let facts = |p: &Path| -> Vec<String> {
        run(&["info", "-v", "-i", p.to_str().unwrap()])
            .lines()
            .map(str::trim)
            .filter(|l| {
                [
                    "SHA1:",
                    "Data SHA1:",
                    "Hunk Size",
                    "Unit Size",
                    "Logical",
                    "Compression",
                    "Metadata:",
                    "CYLS",
                    "TRACK",
                ]
                .iter()
                .any(|k| l.starts_with(k))
            })
            .map(str::to_string)
            .collect()
    };
    // (command, input, extra options)
    let cases: Vec<(&str, &Path, Vec<&str>)> = vec![
        ("createhd", &hd, vec![]),
        ("createhd", &hd, vec!["-c", "zlib"]),
        ("createhd", &hd, vec!["-c", "none"]),
        ("createhd", &hd, vec!["-isb", "4096", "-ib", "1048576"]),
        ("createhd", &hd, vec!["-ss", "2048"]),
        ("createdvd", &dvd, vec![]),
        ("createdvd", &dvd, vec!["-c", "none"]),
        ("createraw", &raw, vec!["-us", "4"]),
        ("createraw", &raw, vec!["-us", "8", "-hs", "8192"]),
        ("createraw", &raw, vec!["-us", "4", "-c", "zstd"]),
    ];
    for (command, input, extra) in cases {
        let ours = scratch("x-ours.chd");
        let theirs = scratch("x-theirs.chd");
        let _ = std::fs::remove_file(&theirs);
        let mut args = vec![
            command,
            "-i",
            input.to_str().unwrap(),
            "-o",
            theirs.to_str().unwrap(),
        ];
        args.extend(&extra);
        run(&args);
        let mut o = CreateData::new(
            match command {
                "createhd" => Media::Hd,
                "createdvd" => Media::Dvd,
                _ => Media::Raw,
            },
            ours.clone(),
        );
        o.input = Some(input.to_path_buf());
        o.force = true;
        let mut it = extra.iter();
        while let Some(&k) = it.next() {
            let v = *it.next().unwrap();
            match k {
                "-c" => {
                    o.codecs = Some(if v == "none" {
                        vec![]
                    } else {
                        vec![Codec::from_tag(v.as_bytes().try_into().unwrap()).unwrap()]
                    })
                }
                "-us" | "-ss" => o.unit_bytes = Some(v.parse().unwrap()),
                "-hs" => o.hunk_bytes = Some(v.parse().unwrap()),
                "-isb" => o.start_byte = Some(v.parse().unwrap()),
                "-ib" => o.bytes = Some(v.parse().unwrap()),
                other => panic!("{other}"),
            }
        }
        rawio::create(&o, |_, _| {}).unwrap();
        assert_eq!(facts(&ours), facts(&theirs), "{command} {extra:?}");
        if Image::open(&ours, None).unwrap().header().compressed() {
            let v = run(&["verify", "-i", ours.to_str().unwrap()]);
            assert!(v.contains("Overall SHA1 verification successful"), "{v}");
        }
    }
}
