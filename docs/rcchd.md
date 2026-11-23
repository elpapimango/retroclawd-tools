# rcchd

Make, unpack, inspect, check and edit CHD files (compressed hunks of data): what MAME's `chdman` does, with its command
names and options. CD images (cue/bin, iso, gdi), hard disks, DVDs and any other data.

```
rcchd info        -i FILE [-v]
rcchd verify      -i FILE [-ip PARENT] [--fix]            exit status 1 on a mismatch
rcchd createcd    -i INPUT.cue|.iso|.gdi -o OUT [-op PARENT] [-c CODECS] [-hs BYTES] [-np N] [-f]
rcchd createdvd   -i INPUT [-isb|-ish N] [-ib|-ih N] -o OUT [-op PARENT] [-c CODECS] [-hs BYTES] [-np N] [-f]
rcchd createhd    [-i INPUT] [-isb|-ish N] [-ib|-ih N] -o OUT [-op PARENT] [-c CODECS] [-hs BYTES]
                  [-ss SECTORBYTES] [-s BYTES] [-chs C,H,S] [-tp TEMPLATE] [-np N] [-f]
rcchd createraw   -i INPUT -us UNITBYTES [-isb|-ish N] [-ib|-ih N] -o OUT [-op PARENT] [-c CODECS] [-hs BYTES]
rcchd extractcd   -i FILE [-ip PARENT] -o OUT.cue|.gdi [-ob BIN] [-sb] [-f]
rcchd extractdvd | extracthd | extractraw  -i FILE [-ip PARENT] [-isb|-ish N] [-ib|-ih N] -o OUT [-f]
rcchd copy        -i FILE [-ip PARENT] -o OUT [-op PARENT] [-c CODECS] [-hs BYTES] [-np N] [-f]
rcchd addmeta     -i FILE -t TAG [-ix N] (-vt TEXT | -vf FILE) [-nocs]
rcchd delmeta     -i FILE -t TAG [-ix N]
rcchd dumpmeta    -i FILE -t TAG [-ix N] [-o FILE] [-f]
rcchd listtemplates
```

The short options of more than one letter (`-ip`, `-hs`, `-np`, ...) are chdman's, and are also available as long options
(`--inputparent`, `--hunksize`, `--numprocessors`, ...). Nothing overwrites an existing file without `-f` / `--force`.
`addmeta`, `delmeta` and `verify --fix` change the file in place.

## Codecs

`-c` takes up to four, comma separated, or `none`: `zlib`, `zstd`, `lzma`, `huff`, `flac` for data, `cdzl`, `cdzs`, `cdlz`,
`cdfl` for CD images. Each hunk is stored with the one that makes it smallest, as it is when none does, or as a reference to
an earlier identical hunk (or to the parent's). Defaults are chdman's: `cdlz,cdzl,cdfl` for a CD (hunks of eight frames,
19 584 bytes), `lzma,zlib,huff,flac` for the others (hunks of 4096 bytes, or as many units as fit in them), none for a blank
disk.

## What is checked against chdman

`chdman` 0.289 is used as a black box (never read for code): the files made here pass `chdman verify`, carry the same data and
overall SHA-1 and the same metadata as the ones chdman makes from the same input and options (uncompressed files are byte for
byte the same), `rcchd extractcd` writes the same cue, gdi and bin files, and files in both directions read each other,
parents included. `crates/rcchd/tests` has the checks (`RC_CHDMAN=path cargo test -p rcchd -- --ignored` runs the ones that
need chdman). Hashes of the same disc agree whatever codec made the file, so a CHD from here and one from chdman are the same
disc.

## Media

- **CD** (`createcd`, `extractcd`): a cue sheet (`FILE` BINARY or MOTOROLA, `TRACK`, `INDEX 00/01`, `PREGAP`, `POSTGAP`), an
  `.iso` (one `MODE1` track) or a `.gdi`. Track types `MODE1`, `MODE1_RAW`, `MODE2`, `MODE2_FORM1`, `MODE2_FORM2`,
  `MODE2_FORM_MIX`, `MODE2_RAW`, `AUDIO`; audio is kept big-endian in the CHD and turned back on extraction. A pregap in the
  file (`INDEX 00`) is stored with the track and written back as `INDEX 00`; a `PREGAP` command is not stored and comes back as one
  (chdman marks the stored kind with a `V` in `PGTYPE`). Each track is padded with zero frames to a multiple of four.
- **GD-ROM** (`.gdi`): the CHD keeps the whole LBA space (`CHGD` metadata: each track runs to the next one's first LBA, with `PAD`
  frames of zeros after its data); extraction writes `name01.bin`, `name02.raw`, ... and the `.gdi`.
- **Hard disk** (`createhd`, `extracthd`): `GDDD` metadata with the geometry (guessed from the size as chdman does, or `-chs`, or a
  drive from `listtemplates`) and the sector size (`-ss`, default 512). Without an input file `-s`, `-chs` or `-tp` makes a blank,
  uncompressed disk.
- **DVD** (`createdvd`, `extractdvd`): 2048-byte sectors, hunks of two, `DVD ` metadata.
- **Raw** (`createraw`, `extractraw`): any file cut into units of `-us` bytes; no metadata.
- **Delta files**: `-op PARENT` stores only the hunks the parent does not have; reading one needs the parent (`-ip`), whose SHA-1 is
  checked.

## Not done

Laser disc images (`createld`, `extractld`: they carry video, the `avhu` codec), CHD versions 1 to 4, CD subchannel data (a track
that has it is refused on extraction and is made without it), `.toc`, `.nrg` and WAV/MP3-based cue sheets, `info` output
identical to chdman's text (the facts are the same, the layout is not). Writing: the Zstandard encoder is the fast level of the
pure-Rust `ruzstd` and FLAC is a simple fixed-predictor encoder, so those two codecs compress a little less than chdman's.
