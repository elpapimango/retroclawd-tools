//! CD images through `createcd` and `extractcd`: what goes in comes out, whichever codecs hold it in between. With `RC_CHDMAN` set
//! (the path of chdman, used as a black box) the files must also verify there and carry the same hashes chdman makes.

use chd::{Codec, Image};
use rcchd::createcd::{self, CreateCd};
use rcchd::extractcd::{self, ExtractCd};
use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rcchd-cd-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// A disc of a data track, an audio track with a stored pregap (INDEX 00) and an audio track with a virtual one (PREGAP), in three files.
fn make_disc(tag: &str) -> PathBuf {
    let mut seed = 99u32;
    let mut noise = move || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (seed >> 24) as u8
    };
    let mut data = Vec::new();
    for s in 0..40u32 {
        let mut sector = vec![0u8; 2352];
        sector[..12].copy_from_slice(&[0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0]);
        sector[12..16].copy_from_slice(&[0, 2, s as u8, 1]);
        for (i, b) in sector[16..2064].iter_mut().enumerate() {
            *b = if s % 3 == 0 { noise() } else { (i / 8) as u8 };
        }
        data.extend(sector);
    }
    let mut audio = Vec::new();
    for n in 0..588 * 70u32 {
        let l = ((f64::from(n) * 0.031).sin() * 9000.0) as i16;
        let r = ((f64::from(n) * 0.047).sin() * 7000.0) as i16;
        audio.extend(l.to_le_bytes());
        audio.extend(r.to_le_bytes());
    }
    let more: Vec<u8> = (0..2352 * 9).map(|i| (i % 253) as u8).collect();
    std::fs::write(scratch(&format!("{tag}-d.bin")), &data).unwrap();
    std::fs::write(scratch(&format!("{tag}-a.bin")), &audio).unwrap();
    std::fs::write(scratch(&format!("{tag}-b.bin")), &more).unwrap();
    let cue = scratch(&format!("{tag}.cue"));
    std::fs::write(
        &cue,
        format!(
            "FILE \"{tag}-d.bin\" BINARY\n  TRACK 01 MODE1/2352\n    INDEX 01 00:00:00\n\
             FILE \"{tag}-a.bin\" BINARY\n  TRACK 02 AUDIO\n    INDEX 00 00:00:00\n    INDEX 01 00:00:20\n\
             FILE \"{tag}-b.bin\" BINARY\n  TRACK 03 AUDIO\n    PREGAP 00:02:00\n    INDEX 01 00:00:00\n"
        ),
    )
    .unwrap();
    cue
}

fn read(p: &Path) -> Vec<u8> {
    std::fs::read(p).unwrap()
}

fn create(cue: &Path, out: &Path, codecs: Vec<Codec>, hunk: u32) -> chd::Summary {
    let mut o = CreateCd::new(cue.to_path_buf(), out.to_path_buf());
    o.codecs = codecs;
    o.hunk_bytes = hunk;
    o.force = true;
    createcd::create(&o, |_, _| {}).unwrap()
}

#[test]
fn every_codec_set_round_trips_the_three_tracks() {
    let cue = make_disc("rt");
    let sets: Vec<Vec<Codec>> = vec![
        vec![],
        vec![Codec::CdZlib],
        vec![Codec::CdLzma],
        vec![Codec::CdZstd],
        vec![Codec::CdFlac],
        vec![Codec::CdLzma, Codec::CdZlib, Codec::CdFlac],
    ];
    for codecs in sets {
        let out = scratch("rt.chd");
        create(&cue, &out, codecs.clone(), 19584);
        let mut image = Image::open(&out, None).unwrap();
        assert!(image.verify(|_, _| {}).unwrap().ok(), "{codecs:?}");
        // three tracks: 40 frames, 70 (20 of stored pregap and 50 of audio), 9; each padded to a multiple of four in the file
        let tracks = extractcd::tracks_of(&image).unwrap();
        let frames: Vec<u32> = tracks.iter().map(|(t, _)| t.frames).collect();
        assert_eq!(frames, [40, 70, 9], "{codecs:?}");
        assert_eq!(tracks[1].1, 40);
        assert_eq!(tracks[2].1, 40 + 72);
        let x = ExtractCd {
            input: out.clone(),
            parent: None,
            output: scratch("rt-x.cue"),
            output_bin: Some(scratch("rt-x.bin")),
            split: false,
            force: true,
        };
        extractcd::extract(&x, |_, _| {}).unwrap();
        // one bin: the three files one after the other
        let mut want = read(&scratch("rt-d.bin"));
        want.extend(read(&scratch("rt-a.bin")));
        want.extend(read(&scratch("rt-b.bin")));
        assert!(
            read(&scratch("rt-x.bin")) == want,
            "{codecs:?}: the bin differs"
        );
        let sheet = String::from_utf8(read(&scratch("rt-x.cue")))
            .unwrap()
            .replace("\r\n", "\n");
        assert!(
            sheet.contains("PREGAP 00:02:00") && sheet.contains("INDEX 00 00:00:40"),
            "{sheet}"
        );
        assert!(sheet.contains("INDEX 01 00:00:60"), "{sheet}");
    }
}

#[test]
fn an_iso_becomes_one_mode1_track_and_split_bins_are_made_per_track() {
    let iso = scratch("a.iso");
    std::fs::write(
        &iso,
        (0..2048 * 50)
            .map(|i| (i * 7 % 256) as u8)
            .collect::<Vec<u8>>(),
    )
    .unwrap();
    let out = scratch("iso.chd");
    let mut o = CreateCd::new(iso.clone(), out.clone());
    o.force = true;
    createcd::create(&o, |_, _| {}).unwrap();
    let image = Image::open(&out, None).unwrap();
    let t = extractcd::tracks_of(&image).unwrap();
    assert_eq!(t.len(), 1);
    assert_eq!(
        t[0].0.text(),
        "TRACK:1 TYPE:MODE1 SUBTYPE:NONE FRAMES:50 PREGAP:0 PGTYPE:MODE1 PGSUB:NONE POSTGAP:0"
    );
    let x = ExtractCd {
        input: out.clone(),
        parent: None,
        output: scratch("iso-x.cue"),
        output_bin: None,
        split: true,
        force: true,
    };
    extractcd::extract(&x, |_, _| {}).unwrap();
    assert_eq!(read(&scratch("iso-x (Track 1).bin")), read(&iso));
    // an existing output is not replaced without force
    let again = ExtractCd {
        input: out,
        parent: None,
        output: scratch("iso-x.cue"),
        output_bin: None,
        split: true,
        force: false,
    };
    assert!(extractcd::extract(&again, |_, _| {}).is_err());
}

#[test]
fn mistakes_are_refused_in_words() {
    let cue = make_disc("bad");
    let mut o = CreateCd::new(cue, scratch("bad.chd"));
    o.hunk_bytes = 4096;
    o.force = true;
    assert!(
        createcd::create(&o, |_, _| {}).is_err(),
        "a CD hunk is whole frames"
    );
    o.hunk_bytes = 19584;
    o.codecs = vec![Codec::Zlib];
    assert!(
        createcd::create(&o, |_, _| {})
            .unwrap_err()
            .0
            .contains("CD codecs")
    );
}

#[test]
#[ignore]
fn chdman_verifies_our_files_and_makes_the_same_hashes() {
    let Ok(tool) = std::env::var("RC_CHDMAN") else {
        return;
    };
    let cue = make_disc("cm");
    let ours = scratch("cm-ours.chd");
    let theirs = scratch("cm-theirs.chd");
    let _ = std::fs::remove_file(&theirs);
    create(
        &cue,
        &ours,
        vec![Codec::CdLzma, Codec::CdZlib, Codec::CdFlac],
        19584,
    );
    let run = |args: &[&str]| {
        let out = Command::new(&tool).args(args).output().unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let verify = run(&["verify", "-i", ours.to_str().unwrap()]);
    assert!(
        verify.contains("Overall SHA1 verification successful"),
        "{verify}"
    );
    run(&[
        "createcd",
        "-i",
        cue.to_str().unwrap(),
        "-o",
        theirs.to_str().unwrap(),
    ]);
    let hashes = |p: &Path| -> Vec<String> {
        run(&["info", "-v", "-i", p.to_str().unwrap()])
            .lines()
            .filter(|l| {
                l.starts_with("SHA1:") || l.starts_with("Data SHA1:") || l.contains("TRACK:")
            })
            .map(|l| l.trim().to_string())
            .collect()
    };
    assert_eq!(
        hashes(&ours),
        hashes(&theirs),
        "chdman made other data or metadata from the same cue"
    );
}
