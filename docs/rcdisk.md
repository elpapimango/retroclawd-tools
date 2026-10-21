# rcdisk

```
rcdisk list      IMAGE [--json]
rcdisk info      IMAGE [--json]
rcdisk validate  IMAGE [--json]              exit status 1 when there are errors
rcdisk extract   IMAGE [NAMES...] [-d DIR | -o FILE] [--keep-header] [--force]
rcdisk add       IMAGE FILES... [--name N] [--type T] [--replace] [-o OUT] [--force]
                                [--user N] [--header amsdos|plus3dos] [--load A] [--entry A]   (CP/M)
rcdisk rm        IMAGE NAMES... [-o OUT] [--force]
rcdisk rename    IMAGE OLD NEW [-o OUT] [--force]
rcdisk create    OUTPUT [-f FORMAT] [-n NAME] [--id XY] [--force]
rcdisk convert   INPUT OUTPUT [--to FORMAT] [--force]
rcdisk formats
```

`add`, `rm` and `rename` change IMAGE in place (the file is replaced atomically); `-o` writes a copy instead. Nothing
else overwrites an existing file without `--force`. Names may be patterns: `*` (any run of characters) and `?`
(one character); on CBM disks `*` ends the comparison as the DOS does. Addresses take `0x4000`, `$4000` or `16384`.

## Formats

| Format | Read | Write | Notes |
|---|---|---|---|
| `.d64` (35 or 40 tracks, with or without error info) | yes | yes | 1541 |
| `.x64` | yes | yes | a `.d64` behind a 64-byte header; `convert` to and from `.d64` changes only the container |
| `.d71`, `.d81` | yes | yes | 1571, 1581 |
| `.g64` | yes | no | the standard sectors are decoded; `convert --to d64` keeps those (copy protection is lost, and said so) |
| `.dsk` / `.edsk` | yes | yes | CPCEMU standard and extended; CP/M with AMSDOS (CPC) or +3DOS (Spectrum +3) |

`create --format`: `d64`, `d64-40`, `x64`, `d71`, `d81`, and for CP/M `dsk` (CPC data format, 178 KB free),
`dsk-system` (CPC system format), `dsk-ibm`, `dsk-plus3` (+3, 173 KB free). The format is otherwise taken from the
output's extension.

## Commodore disks

`list` prints the directory the way the DOS does (`blocks "NAME" TYPE`, flags `<locked,unclosed>`); `--json` has
name, name as hex (PETSCII names can print alike), type, blocks and flags. Names are shown in ASCII: unshifted
letters stay upper case, shifted ones become lower case, anything else unprintable `?`; typed names are made upper case.

* `add`: type `prg`, `seq` or `usr` (`--type`, default from the extension: `.seq`/`.txt` -> seq, `.usr` -> usr, else
  prg); the name defaults to the file name without extension and must fit 16 characters (`--name`). The file is stored
  as it is (a PRG already starts with its load address). `--replace` is the `@0:` form.
* `rm`, `rename`: as `SCRATCH` and `RENAME`; a locked file is refused.
* `validate` follows the directory chain and every file's chain and compares the BAM with what is really in use:
  loops, chains leaving the disk, sectors shared by two files, files the BAM thinks are free, sectors that are
  allocated but belong to nothing, unclosed files, block counts that disagree with the chain.
* `convert` between `.d64`/`.d71`/`.d81` copies the files (disk name and ID kept). REL and partition entries cannot
  be carried over and are reported; so is a file that does not fit.

## CP/M disks (CPC and +3)

The file system is CP/M's: 8.3 names (upper case), user areas 0-15 (`3:NAME.EXT`), read-only/system attributes,
1 KB blocks, a 64-entry directory. CP/M stores whole 128-byte records, so a file's exact length lives in the
**header** AMSDOS or +3DOS put in front of the data:

* `list` shows `BAS`/`BIN`/`ASC` (AMSDOS: with `load=&4000,entry=&4010` for binaries) or `P3-PRG`/`P3-COD`/... (+3DOS),
  and the exact size when a header says it; a file without a header shows its record-padded CP/M size and `ASC`.
* `extract` strips a valid header and cuts to the length it records, so the bytes you added come back exactly;
  `--keep-header` gives the raw file (header, data, padding to 128 bytes).
* `add --header amsdos --load ADDR [--entry ADDR] [--type bin|bas]` writes an AMSDOS header (a BASIC program gets load
  address `&170`); `--header plus3dos` a +3DOS one; without `--header` the data is stored as it is.
* Geometry is recognised from the sector IDs of track 0 (`$C1` data, `$41` system, `$01` +3 or IBM) or from a +3
  disk specification in the first sector (so other layouts can be read; writing is limited to the standard 1 KB-block
  layout). Only side 0 of a double-sided image is used.
* `convert` to or from a CP/M disk goes file by file: names are made to fit (8.3, `_` for anything else, `~1` for
  clashes -- reported), headers are dropped (the data is kept).

## Checked

Tests make every format, put files on it, take them off again and convert; the real `.d64` images of the machine
repositories validate clean and survive an extract/add round trip. Disks made, converted and edited by `rcdisk`
were loaded by the emulator's real 1541 DOS (true drive). A `.dsk` made by `rcdisk create` with a binary added
`--header amsdos` was run in the emulator on a CPC6128: real AMSDOS's `CAT` lists it with the size and free space
`rcdisk` computes, and `RUN"DISC` loads and runs it (a program assembled with `rcasm` that writes to memory).

## Atari ST `.st` (FAT12)

`rcdisk create game.st` (or `--format st-ss` for a single-sided 360 KB disk) makes a blank disk with the BPB of
*Atari ST Internals* p.178 (512-byte sectors, 2 per cluster, 2 FATs of 5 sectors, 112 root entries). `add`, `list`,
`extract`, `rm`, `rename` and `validate` work on the root directory (8.3 names, stored upper case; no
subdirectories yet). `rcdisk boot game.st code.bin` makes the boot sector executable: a `BRA.S` to offset 30, the
raw 68000 code there (at most 480 bytes), and the checksum word so the sector sums to `$1234` (*Internals* p.179).
The FAT12 rules (entry packing, end of chain, directory entries, legal names) follow Microsoft's *FAT: General
Overview of On-Disk Format* v1.03; a chain that loops, leaves the disk, or is shorter than the file's size is an
error, never a panic.
