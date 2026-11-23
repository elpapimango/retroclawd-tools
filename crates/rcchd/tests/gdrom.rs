//! GD-ROM images (`.gdi`): the tracks of the whole LBA space, data first and padding after, and back to the same files. With
//! `RC_CHDMAN` set the file must also verify in chdman and carry the same hashes and metadata as chdman makes from the same sheet.

use chd::Image;
use rcchd::createcd::{self, CreateCd};
use rcchd::extractcd::{self, ExtractCd};
use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rcchd-gd-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn audio(sectors: usize, seed: u8) -> Vec<u8> {
    (0..2352 * sectors)
        .map(|i| ((i * 31 + i / 7) as u8).wrapping_add(seed))
        .collect()
}

fn data(sectors: usize, seed: u8) -> Vec<u8> {
    let mut out = Vec::new();
    for k in 0..sectors {
        let mut s = vec![0u8; 2352];
        s[..12].copy_from_slice(&[0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0]);
        s[12..16].copy_from_slice(&[0, 2, k as u8, 1]);
        for (i, b) in s[16..2064].iter_mut().enumerate() {
            *b = (i + k + usize::from(seed)) as u8;
        }
        out.extend(s);
    }
    out
}

/// Four tracks: data at 0, audio at 600, data at 45000, audio at 45100 (the low and high density areas of a GD-ROM).
fn make_gdi(tag: &str) -> PathBuf {
    let files = [
        (format!("{tag}01.bin"), data(10, 1)),
        (format!("{tag}02.raw"), audio(12, 2)),
        (format!("{tag}03.bin"), data(14, 3)),
        (format!("{tag}04.raw"), audio(9, 4)),
    ];
    for (name, bytes) in &files {
        std::fs::write(scratch(name), bytes).unwrap();
    }
    let gdi = scratch(&format!("{tag}.gdi"));
    std::fs::write(
        &gdi,
        format!(
            "4\n1 0 4 2352 {tag}01.bin 0\n2 600 0 2352 {tag}02.raw 0\n3 45000 4 2352 {tag}03.bin 0\n4 45100 0 2352 {tag}04.raw 0\n"
        ),
    )
    .unwrap();
    gdi
}

#[test]
fn a_gdi_fills_the_lba_space_and_comes_back_file_for_file() {
    let gdi = make_gdi("gd");
    let chd = scratch("gd.chd");
    let mut o = CreateCd::new(gdi, chd.clone());
    o.force = true;
    createcd::create(&o, |_, _| {}).unwrap();
    let mut image = Image::open(&chd, None).unwrap();
    assert!(image.verify(|_, _| {}).unwrap().ok());
    let tracks = extractcd::tracks_of(&image).unwrap();
    let shape: Vec<(u32, u32, Option<u32>)> = tracks
        .iter()
        .map(|(t, _)| (t.number, t.frames, t.pad))
        .collect();
    assert_eq!(
        shape,
        [
            (1, 600, Some(590)),
            (2, 44400, Some(44388)),
            (3, 100, Some(86)),
            (4, 9, Some(0))
        ]
    );
    assert_eq!(&image.metadata()[0].tag, b"CHGD");
    assert_eq!(image.header().logical_bytes, 45112 * 2448);

    let x = ExtractCd {
        input: chd,
        parent: None,
        output: scratch("gd-x.gdi"),
        output_bin: None,
        split: false,
        force: true,
    };
    extractcd::extract(&x, |_, _| {}).unwrap();
    for (n, ext) in [("01", "bin"), ("02", "raw"), ("03", "bin"), ("04", "raw")] {
        let got = std::fs::read(scratch(&format!("gd-x{n}.{ext}"))).unwrap();
        let want = std::fs::read(scratch(&format!("gd{n}.{ext}"))).unwrap();
        assert!(got == want, "track {n}");
    }
    let text = String::from_utf8(std::fs::read(scratch("gd-x.gdi")).unwrap())
        .unwrap()
        .replace("\r\n", "\n");
    assert_eq!(
        text,
        "4\n1 0 4 2352 gd-x01.bin 0\n2 600 0 2352 gd-x02.raw 0\n3 45000 4 2352 gd-x03.bin 0\n4 45100 0 2352 gd-x04.raw 0\n"
    );
}

#[test]
fn tracks_that_overlap_are_refused() {
    let gdi = make_gdi("ov");
    // the second track starts before the first one's ten sectors are over
    std::fs::write(&gdi, "2\n1 0 4 2352 ov01.bin 0\n2 5 0 2352 ov02.raw 0\n").unwrap();
    let mut o = CreateCd::new(gdi, scratch("ov.chd"));
    o.force = true;
    assert!(
        createcd::create(&o, |_, _| {})
            .unwrap_err()
            .0
            .contains("next track")
    );
}

#[test]
#[ignore]
fn chdman_agrees_on_a_gdi() {
    let Ok(tool) = std::env::var("RC_CHDMAN") else {
        return;
    };
    let gdi = make_gdi("cm");
    let (ours, theirs) = (scratch("cm-ours.chd"), scratch("cm-theirs.chd"));
    let _ = std::fs::remove_file(&theirs);
    let mut o = CreateCd::new(gdi.clone(), ours.clone());
    o.force = true;
    createcd::create(&o, |_, _| {}).unwrap();
    let run = |args: &[&str]| {
        let out = Command::new(&tool).args(args).output().unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    assert!(
        run(&["verify", "-i", ours.to_str().unwrap()])
            .contains("Overall SHA1 verification successful")
    );
    run(&[
        "createcd",
        "-i",
        gdi.to_str().unwrap(),
        "-o",
        theirs.to_str().unwrap(),
    ]);
    let facts = |p: &Path| -> Vec<String> {
        run(&["info", "-v", "-i", p.to_str().unwrap()])
            .lines()
            .map(str::trim)
            .filter(|l| {
                l.starts_with("SHA1:")
                    || l.starts_with("Data SHA1:")
                    || l.starts_with("TRACK:")
                    || l.starts_with("Total Units")
            })
            .map(str::to_string)
            .collect()
    };
    assert_eq!(facts(&ours), facts(&theirs));
}
