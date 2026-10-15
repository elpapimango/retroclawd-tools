# rcld

```
rcld build     LAYOUT [-o OUTPUT] [--map FILE] [--symbols FILE] [-D NAME=VALUE]... [--force]
rcld template  c64|spectrum|cpc
```

`rcld` is the last step of the tool chain: it takes what `rcasm` makes (or raw binaries and `.prg` files), puts it in
memory, **checks** that nothing overlaps, leaves its region or sits on a reserved range, and writes the image the
output's extension asks for. There are no object files: the sources named in a layout are assembled together as one
program (one symbol space, so they can call each other freely), then placed.

A layout is a TOML file; every path in it is relative to the layout file, and `rcld template` prints a starting
point. Nothing overwrites an existing file without `--force`.

```toml
machine = "c64"                  # c64 | spectrum | cpc | generic (needs cpu = "6502" or "z80")
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
