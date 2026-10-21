# retroclawd-tools

Command-line tools for the machines [retroclawd](https://github.com/elpapimango/retroclawd) emulates (ZX81, ZX
Spectrum, Amstrad CPC, Commodore 64/128): assemble, disassemble, and -- as they arrive -- open, create and
convert tape and disk images. Rust, Linux / macOS / Windows, no emulation inside: these are code and file
tools. The tools build **on their own**; the emulator application does not build or pin this repository.

| Tool | What it does | State |
|---|---|---|
| [`rcasm`](docs/rcasm.md) | two-pass assembler for the 6502 and the Z80, one shared syntax; flat binary, `.prg`, symbol file, listing | works |
| [`rcdis`](docs/rcdis.md) | disassembler for the 6502 and the Z80 that writes source `rcasm` assembles back to the same bytes | works |
| [`rcdisk`](docs/rcdisk.md) | list, info, validate, extract, add, rm, rename, create and convert disk images: Commodore `.d64`/`.x64`/`.d71`/`.d81` (`.g64` read), CPC / +3 `.dsk` with AMSDOS and +3DOS headers; Atari `.atr` info, blank creation and raw sectors (no filesystem) | works |
| [`rctape`](docs/rctape.md) | inspect/edit Spectrum `.tzx`/`.tap`, extract Commodore `.t64`, inspect C64 `.tap`, validate/normalize VIC-20 TAP-v0/v1 and C16/Plus4 TAP-v2; `from-prg` creates VIC-20 KERNAL tapes | works |
| [`rcsnap`](docs/rcsnap.md) | info, convert, dump, extract and create: ZX Spectrum `.z80`/`.sna`/`.szx` into each other, ZX81 `.p`/`.p81`, CPC `.sna`, C64 `.crt`, CPC Plus `.cpr` | works |
| [`rcld`](docs/rcld.md) | place assembled programs (sources, binaries, `.prg`), check overlaps and regions, and write a runnable image: C64 `.prg`/`.d64`/`.d81`/`.crt` (with a `SYS` line), Spectrum `.tap`/`.tzx` (with a BASIC loader) and snapshots, CPC `.dsk`, Atari `.xex` build/inspect | works |

## Build

Reference capture: `scripts/capture_atari800_screen.py` drives Atari800 7.2.1
Win64 through its monitor and native screenshot key, using explicit Atari 800
OS/BASIC replacements and no ROM patches. It supports cold boot and a tools-built
raw Program, with optional memory goldens. It does not yet drive printable
reference keyboard input or record reference audio. Windows/standard-library only;
run `python scripts/capture_atari800_screen.py --help` for arguments.

```
cargo build --release --workspace      # target/release/rcasm, target/release/rcdis
cargo test --workspace
```

The instruction knowledge comes from the retroclawd crates (`z80`, `mos6502`, `machine`), pinned by git
revision in the root `Cargo.toml`. Those repositories are public; cargo fetches them from GitHub (set
`CARGO_NET_GIT_FETCH_WITH_CLI=true` in CI if your git setup needs it).

## A taste

```
$ cat hello.asm
        .cpu 6502
        .org $C000
start:  ldx #0
loop:   lda msg,x
        beq done
        jsr $FFD2
        inx
        bne loop
done:   rts
msg:    .text "HELLO", 13
        .byte 0

$ rcasm hello.asm -f prg -o hello.prg -s hello.sym -l hello.lst
rcasm: 21 bytes ($C000-$C014)

$ rcdis hello.prg --cpu 6502 --follow -s hello.sym      # source rcasm assembles back to the same bytes
```

`rcasm` writes symbol files in the format the emulator's debugger reads (`retroclawd c64 --symbols hello.sym`),
and `rcdis` reads them to name addresses.

## Disks

```
$ rcdisk create game.dsk                                   # a CPC data disk
$ rcdisk add game.dsk disc.bin --header amsdos --load 0x4000 --name DISC.BIN
$ rcdisk list game.dsk
DISC.BIN            27  BIN <load=&4000,entry=&4000>
177 KB free.
$ rcdisk convert game.dsk game.d64                         # file by file, between families
```

## Tapes

```
$ rctape add game.tzx loader.bin --name LOADER --load 0x8000
$ rctape list game.tzx
  1  Standard speed data      19  Bytes: LOADER start 32768, length 300
  2  Standard speed data     302  Data (300 bytes)
$ rctape cleanup game.tzx --all                            # strip info blocks, fix checksums, join pauses
$ rctape split collection.tzx -d programs                  # one file per program
$ rctape convert game.tzx game.tap
```

## Snapshots and cartridges

```
$ rcsnap create game.sna --load 0x8000:game.bin           # a 48K snapshot that starts at 0x8000
$ rcsnap convert game.sna game.z80                         # says what the target cannot keep
$ rcsnap dump game.z80 --range 0x8000:0x100 -o part.bin
$ rcsnap convert rom.bin cart.crt --name MYCART            # a plain 8K / 16K C64 cartridge
```

## Linking

```
$ rcld template c64 > layout.toml        # edit it, then:
$ rcld build layout.toml --map game.map  # assemble, place, check, write game.d64
$ rcld build layout.toml -o game.crt     # the same program as a cartridge
```

## Checked against real ROMs

`rcdis` output reassembles byte-for-byte for the ZX Spectrum, ZX81 and CPC464 ROMs and the C64 KERNAL / BASIC and
1541 DOS ROMs, and for random bytes on both CPUs (which reaches every opcode, undocumented ones and prefixes
included). Programs built with `rcasm` run in the emulator and leave the expected bytes in memory, and disks made by `rcdisk` are read by the emulator's real 1541 DOS and by real AMSDOS, tapes made by `rctape` are loaded by the Spectrum ROM's own tape loader, snapshots made or converted by `rcsnap` (and a cartridge) start in the emulator, and programs linked by `rcld` start on the C64, Spectrum and CPC (through the real BASIC loaders). These tests
need the ROM images and a built emulator, so they run only when told where they are (see `CLAUDE.md`).

See `CLAUDE.md` for the layout and conventions, `CREDITS.md` for the references.

## Machine scenarios

`rc-scenarios check <machine>.manifest.toml` validates a scenario manifest without
firmware. `rc-scenarios run <manifest>` runs it with `RETROCLAWD_BIN`, the machine
plugin directory, `RC_ROMS` and `RC_GOLDENS` configured (see `CLAUDE.md`).

Every captured golden requires its real SHA-256. Before a reference capture is
available, a `screen` or `memory` scenario may use `status = "known-gap"` with a
nonempty `gap.reason` and `gap.evidence`, omitting the golden and hash together.
The report explicitly says the comparison was not run; it is never counted as
passed. Memory ranges remain required. Adding a golden restores the normal
comparison rules, including failure when a known gap starts passing.

## License

Free software under the **GNU General Public License v3.0 or later** (see `LICENSE`). No ROM image or commercial
software is part of this repository; the tests that need them (`RC_ROMS`) skip when it is unset.

## Contributing / issues

Not accepted right now -- this is a personal work-in-progress project. Pull requests and issues will be closed
unreviewed.
