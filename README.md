# retroclawd-tools

A tool suite for writing programs for retro computers and consoles: assemble, link, package, convert and inspect. It
turns source files into the machine's own file (a tape, disk, cartridge, ROM or executable), runs it in
[retroclawd](https://github.com/elpapimango/retroclawd) and gives the debugger its symbols. Rust; Linux, macOS and
Windows; no emulation inside. It is released on its own: `cargo install rcb rcasm rcld ...`, or the archives of
the [releases](https://github.com/elpapimango/retroclawd-tools/releases).

## What it can build

| Machine | CPU | `rcb new --machine` | Image |
|---|---|---|---|
| Commodore 64 | 6502 | `c64` | `.prg` (with a `SYS` line), `.d64`, `.d81`, `.crt` |
| ZX Spectrum | Z80 | `spectrum` | `.tap`, `.tzx` (with a BASIC loader), `.sna`, `.z80`, `.szx` |
| Amstrad CPC | Z80 | `cpc` | `.dsk` |
| Atari 8-bit | 6502 | `atari` | `.xex`, `.car` |
| Atari ST | 68000 | `atari-st` | `.prg` |
| NEC PC Engine | HuC6280 | `pcengine` | `.pce` HuCard (banked) |
| NES / Famicom | 6502 | `nes` | `.nes` (iNES / NES 2.0, banked) |
| Super Nintendo | 65816 (+ SPC700 code) | `snes` | `.sfc` LoROM, HiROM, ExHiROM |
| Sega Mega Drive | 68000 | `genesis` | `.md` ROM with vectors, header, checksum |
| SNK Neo Geo | 68000 | `neogeo` | cartridge set `.zip` |
| Amiga | 68000 | `amiga` | hunk executable, `.adf` disk |
| Sharp X68000 | 68000 | `x68000` | `.x` executable |

Not yet: Mega-CD discs, X68000 floppies with a Human68k filesystem, 68010-68030 / FPU / PMMU targets. See `docs/` and
the plans under `docs/superpowers/plans/`.

## The tools

| Tool | What it does |
|---|---|
| [`rcb`](docs/rcb.md) | the **project driver**: `rcb new`, `build`, `check`, `run`, `debug`, `clean`, `targets` over an `rcb.toml` |
| [`rcasm`](docs/rcasm.md) | assembler: 6502, Z80, HuC6280, 65816, SPC700, 68000. Flat images, `.prg`, relocatable objects (`.rco`), symbol files, listings, debug info; ca65-style scopes, unnamed labels, `.repeat`, macros, conditionals |
| [`rcar`](docs/rcar.md) | object libraries (`.rca`): create, list, extract, add, delete |
| [`rcld`](docs/rcld.md) | linker and packager: places sources, objects, libraries and binaries by a TOML layout (or an ld65 `.cfg`), checks overlaps and regions, writes every image above |
| [`rcdis`](docs/rcdis.md) | disassembler (6502, Z80, HuC6280, 65816, SPC700, 68000) whose output assembles back to the same bytes; linear or following flow |
| [`rcasset`](docs/rcasset.md) | PNG to NES / SNES / Mega Drive / PC Engine tiles, SNES and Mega Drive palettes, the Spectrum screen |
| [`rcdisk`](docs/rcdisk.md) | disk images: Commodore `.d64/.d71/.d81`, CPC / +3 `.dsk`, Atari ST `.st/.msa` and hard disks, Atari `.atr` |
| [`rctape`](docs/rctape.md) | tape images: Spectrum `.tzx/.tap`, C64 `.t64/.tap`, VIC-20, C16/Plus4 |
| [`rcsnap`](docs/rcsnap.md) | snapshots and cartridges: Spectrum `.z80/.sna/.szx`, ZX81, CPC `.sna/.cpr`, C64 `.crt` |
| [`rcchd`](docs/rcchd.md) | CHD files like `chdman`: CD, hard disk, DVD, raw; verified against chdman 0.289 |

File formats of the objects and libraries: [`docs/formats.md`](docs/formats.md).

## Install

```
cargo install rcb rcasm rcld rcar rcdis rcasset rcdisk rctape rcsnap rcchd
```

or build from a checkout: `cargo build --release --workspace` (the programs are in `target/release/`).
`rcb run` and `rcb debug` start `rcd` / `rcdd` from the [retroclawd](https://github.com/elpapimango/retroclawd) emulator
(`RCB_EMULATOR` names another program).

## Quick start: a project

```
$ rcb new hello --machine pcengine
$ cd hello
$ rcb build
rcb: build\hello.pce (131072 bytes)
$ rcb run                                  # starts the emulator on it
```

`rcb new` makes this:

```
hello/
  rcb.toml        the project: the machine, its options, the sources
  src/main.asm    a skeleton that builds as it is: the vectors the machine needs and a loop
```

```toml
# rcb.toml
machine = "pcengine"
entry = "reset"

[project]
name = "hello"
sources = ["src/*.asm"]
```

`rcb build` writes `build/hello.pce`, a map of what went where (`hello.map`) and a symbol file (`hello.sym`) the
debugger reads. [`docs/rcb.md`](docs/rcb.md) lists everything an `rcb.toml` takes.

## Quick start: the tools by hand

```
$ cat hello.asm
        .org $080D
start:  ldx #0
loop:   lda msg,x
        beq done
        jsr $FFD2           ; KERNAL: print a character
        inx
        bne loop
done:   rts
msg:    .text "HELLO", 13
        .byte 0

$ rcasm -c 6502 hello.asm -f prg -o hello.prg -s hello.sym -l hello.lst
$ rcdis -c 6502 hello.prg --follow -s hello.sym       # source that assembles back to the same bytes
$ rcld template c64 > layout.toml                     # a starting layout, then:
$ rcld build layout.toml --map game.map -o game.d64   # assemble, place, check, write a disk (or .crt, .prg ...)
```

### Several files and a library

```
$ rcasm -c 6502 -f object main.asm   -o main.rco
$ rcasm -c 6502 -f object screen.asm -o screen.rco
$ rcar create screen.rca screen.rco
$ rcld build layout.toml              # layout.toml names  obj = "main.rco"  and  lib = "screen.rca"
```

`rcld` links a library member only when something needs a name it defines. Sources can also be named directly
(`src = ["main.asm", "screen.asm"]`: each is assembled to its own object) and placed by an ld65 configuration
(`cfg = "c64.cfg"`). Errors name the file and line, the symbol and the distance:
`main.asm:12: far = branch distance 310 (the target is $1136) does not fit in 1 byte(s)`.

### Other tools

```
$ rcdisk create game.dsk && rcdisk add game.dsk disc.bin --header amsdos --load 0x4000 --name DISC.BIN
$ rctape add game.tzx loader.bin --name LOADER --load 0x8000
$ rcsnap create game.sna --load 0x8000:game.bin
$ rcasset tiles sprites.png -f nes -o sprites.chr
$ rcchd createcd -i game.cue -o game.chd
```

## Examples

[`examples/`](examples/README.md) has a ready project for every machine (`rcb build` in each folder), a program
in two files with a library, and CMake projects (`examples/cmake/RetroClawd.cmake` adds `retroclawd_object`,
`retroclawd_library`, `retroclawd_image` and `retroclawd_project` to any CMake build).

```cmake
list(APPEND CMAKE_MODULE_PATH "${CMAKE_CURRENT_SOURCE_DIR}/examples/cmake")
include(RetroClawd)
retroclawd_object(${CMAKE_BINARY_DIR}/main.rco SOURCE src/main.asm CPU 6502)
retroclawd_image(hello LAYOUT layout.toml OUTPUT ${CMAKE_BINARY_DIR}/hello.prg
                 DEPENDS ${CMAKE_BINARY_DIR}/main.rco)
```

## Layout of the repository

```
crates/    rc-common (shared code, the .rco/.rca/.rcdbg formats), rcasm, rcar, rcld, rcdis, rcb, rcasset,
           rcdisk, rctape, rcsnap, rcchd
docs/      one page per tool, formats.md, and the design and plans under superpowers/
examples/  projects per machine and CMake projects
fuzz/      fuzz targets for the readers and the assembler
```

The instruction knowledge and the file formats come from the crates of
[retroclawd-core](https://github.com/elpapimango/retroclawd-core) (`rc-z80`, `rc-mos6502`, `rc-huc6280`, `rc-w65c816`,
`rc-spc700`, `rc-mc68000`, `rc-ines`, `rc-sfc`, `rc-amiga`, `rc-x68k`, the disk, tape and CHD crates), taken from crates.io.
The assemblers build their tables from the cores' disassemblers, so whatever the debugger shows assembles.

## Develop

```
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

`CLAUDE.md` describes the conventions. Tests that need ROMs or a built emulator skip unless told where they are.

## License

GNU General Public License v3.0 or later (see `LICENSE`). No ROM image or commercial software is part of this
repository. `CREDITS.md` lists the references the work is based on.

## Contributing

Not accepted right now: this is a personal project, and pull requests and issues will be closed unreviewed.
