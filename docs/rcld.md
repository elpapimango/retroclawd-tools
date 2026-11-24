# rcld

```
rcld build     LAYOUT [-o OUTPUT] [--map FILE] [--symbols FILE] [-D NAME=VALUE]... [--force]
rcld inspect   IMAGE.xex
rcld template  c64|spectrum|cpc
```

`rcld` is the last step of the tool chain: it takes what `rcasm` makes (or raw binaries and `.prg` files), puts it in
memory, **checks** that nothing overlaps, leaves its region or sits on a reserved range, and writes the image the
output's extension asks for. There are no object files: the sources named in a layout are assembled together as one
program (one symbol space, so they can call each other freely), then placed.

A layout is a TOML file; every path in it is relative to the layout file, and `rcld template` prints a starting
point. Nothing overwrites an existing file without `--force`.

```toml
machine = "c64"                  # c64 | spectrum | cpc | atari | atari-st | pcengine | generic (needs cpu = "6502", "z80" or "huc6280")
name = "MYGAME"                  # program name on the medium (tape header, disk file, cartridge)
entry = "start"                  # a symbol of the inputs, or an address
output = "mygame.d64"            # the extension chooses the image; `rcld build -o` overrides
fill = 0                         # byte for holes that gaps = "fill" fills
gaps = "error"                   # or "fill": a hole inside a one-piece image

[[input]]
asm = ["main.asm", "player.asm"]     # assembled together, in this order

[[input]]
bin = "music.bin"                    # raw bytes ...
at = 0xC000                          # ... at this address

[[input]]
prg = "sprites.prg"                  # a .prg brings its own load address

[[input]]
symbols = "kernal.sym"               # names the sources can use: NAME = $1234, or VICE `al C:1234 .NAME`

[[region]]                           # with regions, every piece must lie wholly inside one
name = "ram"
start = 0x0801
end = 0x9FFF

[[reserve]]                          # nothing may touch this
start = 0xD000
end = 0xDFFF
why = "I/O"

[[file]]                             # an extra file for a disk image
path = "level1.bin"
name = "LEVEL1"
type = "seq"                         # C64: prg | seq | usr; CP/M: user = N

[define]                             # constants for the sources, like rcasm -D
LEVEL = 3

[options]                            # see the machines below
```

## Atari XEX

Use `machine = "atari"` and an `.xex` output to package the existing 6502
assembler output or addressed raw binaries as disjoint load segments:

```toml
machine = "atari"
output = "hello.xex"
entry = "start"

[[input]]
asm = "hello.asm"

[[input]]
bin = "data.bin"
at = 0x4000
```

`rcld build layout.toml` keeps gaps as separate segments rather than filling
them. Normal region/reserve/overlap checks apply. An explicit `entry` adds a
two-byte RUNAD segment at `$02E0`; without `entry`, no vector is invented.
If inputs already supply RUNAD, omit `entry`; defining both is an overlap error.
No BASIC stub or INITAD execution is involved. Assembler syntax is unchanged.
`rcld inspect hello.xex` reports each ordered segment's inclusive address range
and length, including repeats/overlaps in external files, without executing it.
Library APIs `atari::inspect` and `atari::create` use the published family
`atari-media` crate and preserve supplied segment order.

## Atari CAR

An Atari `.car` output packages a standard 8K cartridge at `$A000-$BFFF`,
or 16K when any input starts below `$A000` within `$8000-$BFFF`. The emitter
checks the ROM window, fills unused bytes with the layout's `fill` value,
and writes the typed CART header and checksum through `atari-media`.
Use `fill = 255` for unconnected ROM bytes.

Supply the native OS entry/init vectors and flags explicitly at `$BFFA-$BFFF`;
unlike `.xex`, the emitter inserts no RUNAD or startup code. XEGS banked
creation and CAS creation are not exposed by this linker.

## Checks

Every error names the pieces: `data.bin ($0810-$081F) overlaps assembled code ($080D-$0826)`, `... is outside every
region (ram $0801-$9FFF)`, `... is in the reserved range $D000-$DFFF (I/O)`. A hole in a one-piece image (a `.prg`,
a cartridge, a CP/M binary) is an error naming its addresses unless `gaps = "fill"`. `--map` writes every piece with
its range and size and how full each region is; `--symbols` writes the labels of the sources in the format
`retroclawd --symbols` reads.

## C64

| Output | What it is |
|---|---|
| `.prg` | load address and bytes. With `basic_stub` (default **on** when the code does not start at $0801 and there is an entry) a line `10 SYS <entry>` is put at $0801 so `RUN` starts the program; code that sits at $0801 is left alone, and a stub asked for on top of it is an overlap error. |
| `.d64`, `.d81` | a formatted disk (`disk_name`, `disk_id`) holding the `.prg` (`file_name`, default `name`) and the `[[file]]` entries |
| `.crt` | a plain 8K or 16K cartridge at $8000 (16K when something lies above $A000). With `cart_header` (default on) the cold and warm start vectors point at the entry and the `CBM80` signature is written at $8004; the code must leave $8000-$8008 free. Nothing may lie outside $8000-$BFFF. |
| `.bin` | the flat image from the lowest address |

## ZX Spectrum

| Output | What it is |
|---|---|
| `.tap`, `.tzx` | a **BASIC loader** (`10 CLEAR n`, one `LOAD ""CODE` per run of code, `RANDOMIZE USR entry`, with the hidden number forms the interpreter reads; header with autostart line 10) followed by the code blocks. `options.screen = "x.scr"` adds a `LOAD ""SCREEN$` (6912 bytes). `options.loader = false` gives bare header + code blocks. The loader needs the memory below $6000 for itself, so code starting lower is an error. `options.pause` is the pause after each block (default 1000 ms). |
| `.sna`, `.z80`, `.szx` | a 48K snapshot that starts at the entry; `options.sp` (default 0), `border`, `im` (default 1), `interrupts` (default off). The format's losses are noted as `rcsnap` does. |
| `.dsk` | a +3 disk with the program as one +3DOS binary file |
| `.bin` | flat image |

Nothing may lie below $4000 (the ROM). The tape name is cut to 10 characters, said in a note.

## Amstrad CPC

`.dsk` is an AMSDOS disk with the program as one binary file `NAME.BIN` (load address = lowest piece, entry from the
layout); `RUN"NAME` starts it. `.bin` is the flat image.

## Checked

`tests/build.rs` builds every output and reads it back with the other tools' libraries (disks with `rcdisk`, tapes with
`rctape`, snapshots and cartridges with `rcsnap`), and covers every check and error above. `tests/emulator.rs` (needs
`RETROCLAWD_BIN`, `RETROCLAWD_MACHINE_PLUGIN_DIRS`, `RC_ROMS`) runs a Spectrum snapshot in all three formats, lets the
Spectrum ROM's own `LD-BYTES` read the loader and the code from a tape `rcld` made, lets the C64 KERNAL load a `.prg`
from a `.d64`, and starts a `.crt`. The parts that need the BASIC interpreter and a keyboard were checked by hand in the
emulator window: a C64 `.prg` with `--run` ran its `SYS` line into the program, a Spectrum tape went through `LOAD ""`,
the loader's `CLEAR`, `LOAD ""CODE` and `RANDOMIZE USR` into the program, and a CPC disk started with `RUN"HELLO`
(each program wrote a marker byte and a string the check read back from memory).

## Not done

* No CPC tape (`.cdt`), no ZX81 `.p`, no CPC Plus `.cpr` output; no 128K snapshots; no bank-switched C64 cartridges.
* Sources are assembled as one program, so there are no separately assembled modules to link; `asm` pieces are all
  named "assembled code" in messages and the map.
* A `.prg` input's BASIC stub is not parsed; a `.tap` on the Spectrum has one program.

## PC Engine / TurboGrafx-16 (`machine = "pcengine"`)

Writes a HuCard `.pce` (or `.tg16`) from HuC6280 sources and raw binaries. `.bank n` before an `.org` in the source puts the code in the
8 KB ROM bank `n`; bank 0 is what the CPU sees at `$E000-$FFFF` after reset, so bank 0 code is assembled there. A piece in bank `n` sits at file offset
`n * 8192 + (address & $1FFF)` and may run on into the next banks. A `bin` input takes `at` and `bank` (default 0).

```toml
machine = "pcengine"
entry = "start"
output = "game.pce"

[[input]]
asm = ["main.asm"]

[[input]]
bin = "graphics.bin"
bank = 1
at = 0x6000

[options]
size = 262144                    # default: the next power of two, at least 128 KB
```

The last ten bytes of bank 0 are the vectors (`$FFF6` IRQ2/BRK, `$FFF8` IRQ1, `$FFFA` timer, `$FFFC` NMI, `$FFFE` reset). Unless the
sources write them, they come from the labels `irq2`, `irq1`, `timer`, `nmi` and `reset` (or `entry`, or `start`); a vector without a label
points at the reset code and `rcld` says so. Sizes are a multiple of 8 KB that is a power of two, 384 KB or 2.5 MB (the card reader mirrors a
power of two and maps the others). Overlaps in the file are errors naming both pieces.


## Objects and libraries

Besides `asm` (one program assembled whole, as above), a layout can link separately assembled code:

```toml
machine = "c64"
entry = "start"

[[input]]
src = ["main.asm", "player.asm"]      # each assembled to its own object (rcasm object mode), then linked

[[input]]
obj = "music.rco"                      # an object made earlier with rcasm -f object

[[input]]
lib = "lib/c64.rca"                    # a library made with rcar: members are linked when a name they define is needed

[[segment]]
name = "CODE"
start = 0x0801                         # or: region = "ram"

[[segment]]
name = "BSS"
start = 0xC000

[[segment]]
name = "ZP"
start = 0x02
```

`rcld` takes the objects as given and pulls library members in for the names still undefined (repeatedly: a member that needs
another member works, in any order). Segments of one name are placed one after another in input order; a segment with no rule
continues after the previous one; the first needs a rule (`start`, or `region` with the layout's regions). A `region` that
a segment does not fit is an error naming both; so is a zero-page segment without a rule. Every relocation is evaluated with the
final addresses and checked for fit; the message names the source line, the expression, the value and the segment:

```
main.asm:12: far = branch distance 310 (the target is $1136) does not fit in 1 byte(s) (segment "CODE" of main.asm)
undefined symbol "putc" (wanted by main.asm)
"putc" is defined in both a.asm and lib.rca(lib.rco)
```

Inputs of the other kinds (`bin`, `prg`, `symbols`, `asm`) can be mixed in. Machines with their own writers (Atari ST, PC Engine)
do not take objects yet.

## NES (`machine = "nes"`)

6502 sources and an iNES file. `[options]`: `mapper`, `submapper`, `mirroring` (`horizontal`, `vertical`, `four`), `battery`, `chr = "tiles.chr"` (the CHR ROM; without it the cartridge has CHR RAM),
`prg_ram`, `nes2`, `banked`, `size`. Flat programs live in `$8000-$FFFF` (a 32 KB PRG; 16 KB when nothing is below `$C000`). With `.bank n` before an `.org` (or `banked = true`) the PRG is 16 KB banks,
bank `n` at file offset `n * 16384 + (address & $3FFF)`; the last bank is the one fixed at `$C000`. The vectors at `$FFFA-$FFFF` come from the labels `nmi`, `reset` (or `start`, or `entry`) and `irq` (or `brk`)
unless the sources write them. Output `.nes`; `rcld template nes`.

## Super Nintendo (`machine = "snes"`)

65816 sources placed by 24-bit `.org`, raw binaries at a 24-bit `at`, output `.sfc`/`.smc`. `[options]`: `mapping` (`lorom` default, `hirom`, `exhirom`), `fast`, `title`, `cart_type`, `ram_size`,
`country`, `version`, `size`, `fix_checksum`. Addresses map to ROM by the mapping (LoROM: `(bank & $7F) * $8000 + (address & $7FFF)` in the `$8000-$FFFF` window; HiROM: banks `$C0-$FF` as they are, or `$00-$3F`/`$80-$BF` upper halves;
ExHiROM as documented in `rcld::snes`); an address outside the ROM window, a piece that runs across the end of a bank window and overlaps are errors naming the piece. The internal header and the vectors
are generated from the labels `reset`, `nmi`, `irq`, `cop`, `brk`, `abort` and the options, and the checksum is computed. If the sources write the header themselves (at `$FFC0` of the mapping's first header bank), it is
kept and only the checksum is fixed. A missing vector label points at the reset code, with a note. `rcld template snes`.

## Amiga (`machine = "amiga"`)

A relocatable 68000 program (sections `.text`, `.data`, `.bss`, as for the Atari ST) written as a hunk executable (no extension or `.exe`) or on an ADF disk (`.adf`: the program, `s/startup-sequence`, the files of `[[file]]`,
a standard boot block). The text is the `CODE` hunk, the data the `DATA` hunk, the bss the `BSS` hunk; every address in the program points into its hunk and is listed in that hunk's `RELOC32`. `[options]`: `memory`
(`any`, `chip`, `fast`), `hunk_symbols`, `fs` (`ofs`/`ffs`), `bootable`, `disk_name`, `file_name`, `startup`. A program starts at the first byte of its first hunk, so `entry` must be the first label.
Facts the Amiga machine has not confirmed yet (the boot block code, the hunk subset) are `[H?]` in `rc-amiga`. `rcld template amiga`.

## X68000 (`machine = "x68000"`)

The same relocatable 68000 program as a Human68k `.x` executable (`entry` is the execution address, default the first byte). A floppy image with a Human68k filesystem is not written yet. `rcld template x68000`.

## Mega Drive / Genesis (`machine = "genesis"`)

68000 sources at absolute addresses (`.org`, the ROM starts at `$000000`; one `.org` per program: pad with `ds.b`) and raw binaries at `at`; output `.md`, `.bin` or `.gen`. When the sources put nothing in `$000000-$0001FF` the
vector table and the header are generated: the vectors from the labels `reset`/`start`/`entry`, `hint`, `vint`, `ext` and the exception names (`bus_error`, `address_error`, `illegal`, `div_zero`, `chk`, `trapv`,
`privilege`, `trace`, `line_a`, `line_f`), the rest pointing at `error` or the reset code; the header from `console`, `title`, `serial`, `copyright`, `region`, `devices`, `stack`. The ROM end and the checksum are always
written (`fix_checksum = false` stops that) and the image is padded to a power of two of at least 128 KB. Not done yet: the Neo Geo cartridge set and the Mega-CD disc.

## Neo Geo (`machine = "neogeo"`)

68000 sources at absolute addresses (the image starts at `$000000`; pad with `ds.b`) and `bin` inputs at `at`, output `.zip`: a cartridge set with the names the emulator reads: `NNN-p1.p1` (a program above 1 MB continues in
`-p2.p2`), and from `[options]` files: `s` (fix layer, `-s1.s1`), `c` (the byte-interleaved sprite region, split into `-c1.c1` even bytes and `-c2.c2` odd bytes), `m` (Z80, `-m1.m1`), `v1`/`v2` (`-v1.v11`, `-v2.v21`);
`ngh` is `NNN` (default 999). The header at `$100` (`NEO-GEO`, the jump table, the security pointer) is the program's own to write; rcld notes when it is missing [H?]. Files are stored, not compressed.

## ld65 configuration files

`cfg = "c64.cfg"` (top level of the layout) reads an ld65 `.cfg`: each `MEMORY` area becomes a region (`start`, `size`), each `SEGMENTS` entry a placement rule (`load` the region, `start`, `align`). `file`, `fill`, `type`, `define`, `run`, `offset`, and the `FILES`, `FORMATS`, `SYMBOLS`, `FEATURES` blocks are accepted and ignored: the layout's `output`, `fill` and machine decide those. Values are numbers with `+ - *`; a linker symbol such as `__STACKSIZE__` is an error. Use it with `src = [...]` inputs (separate compilation): the sources' `.segment "CODE"` / `"RODATA"` / `"BSS"` ... land where the config says.

## 68000 objects and libraries

For the Atari ST, the Amiga, the X68000, the Mega Drive and the Neo Geo, `src`, `obj` and `lib` inputs go through the linker like the 8-bit machines' (see "Objects and libraries"), the sources assembled with `rcasm --cpu 68000 -f object`'s rules.
An `asm` input and these cannot be mixed in one layout. The sections are `TEXT`, `DATA` and `BSS`.

* **Atari ST, Amiga, X68000** (loaders that relocate the program): the sections are placed from 0 in the order TEXT, DATA, BSS, and every four-byte field that holds an address of the program goes in the loader's relocation
  table (the GEMDOS fixups, the hunk relocations). An address in a word or a byte, or an expression that is neither an address nor a constant, is an error that names the line. There are no regions.
* **Mega Drive**: `TEXT` at `$000200` (after the vectors and header the target generates; put `[[segment]] name = "TEXT" start = 0` to write your own), `DATA` after it in the ROM, `BSS` at `$FF0000` in the work RAM.
* **Neo Geo**: `TEXT` at `$000000`, `DATA` after it, `BSS` at `$100000`.

`[[segment]]` rules override these (`name`, `start`, `region`, `align`).

```
rcasm --cpu 68000 -f object main.s -o main.rco
rcasm --cpu 68000 -f object lib.s  -o lib.rco
rcar create libs.rca lib.rco
rcld build layout.toml -o game.prg        # [[input]] obj = ["main.rco"]  [[input]] lib = ["libs.rca"]
```

## Mega-CD (`machine = "segacd"`)

A data disc image, `.iso` (2048-byte sectors), that the Mega-CD BIOS boots. Each `[[input]] asm` is assembled on its own at absolute addresses (`.org`), `bin` inputs go at `at`:
a piece at `$FF0584` and up in the main CPU's work RAM is the **initial program** (IP: the BIOS runs its region security code first and jumps to `$FF0584`), a piece at `$006000` and up is the **sub program**
(SP: a module header "name, version, next, length, entry offset" and a table of entry offsets, as `rcb new --machine segacd` writes it). Sector 0 holds the header (`SEGADISCSYSTEM`, the volume name from `name`, the IP and SP offsets and sizes,
a Genesis-style header at `$100` from `[options] title serial copyright console region`), the IP and SP follow in the system area (sectors 0-15, 32 KB in all), and sector 16 starts an ISO 9660 volume with the `[[file]]` entries
(`path`, `name`: 8.3 upper-case names) that the sub program reads through the BIOS.

The security code at `$200-$783` is Sega's and is not part of this tool: `[options] security = "file"` names a file with it (1412 or 1536 bytes, from a disc you own). Without it the area is zeros and a note says so: the disc is
for emulators and BIOS versions that do not check it. The entry offset and work RAM size fields of the header are written as 0 `[H?]`. The ISO 9660 volume reads back in 7-Zip. Not done: audio tracks, CD-ROM XA sectors, a `.cue` or CHD
(`rcchd createcd` makes a CHD from a cue sheet), subdirectories.
