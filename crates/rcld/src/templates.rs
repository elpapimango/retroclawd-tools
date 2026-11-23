//! Starting layouts, printed by `rcld template <machine>`.

pub fn template(machine: &str) -> Option<&'static str> {
    Some(match machine {
        "c64" => C64,
        "spectrum" => SPECTRUM,
        "cpc" => CPC,
        "atari-st" => ATARI_ST,
        "pcengine" => PCENGINE,
        _ => return None,
    })
}

const C64: &str = r#"# rcld layout for the Commodore 64.  rcld build this-file.toml
machine = "c64"
name = "MYGAME"                  # program name on the disk / cartridge
entry = "start"                  # a label of the sources, or an address
output = "mygame.d64"            # .prg  .d64  .d81  .crt  .bin  (rcld build -o overrides)

[[input]]
asm = ["main.asm"]               # assembled together, in this order, as one program

# [[input]]
# bin = "music.bin"              # a raw binary ...
# at = 0xC000                    # ... at this address
#
# [[input]]
# prg = "sprites.prg"            # a .prg brings its own load address
#
# [[input]]
# symbols = "kernal.sym"         # names the sources can use (NAME = $1234, or VICE "al C:1234 .NAME")

# With regions, every piece must lie inside one of them; reserves are off limits.
[[region]]
name = "ram"
start = 0x0801
end = 0x9FFF

# [[reserve]]
# start = 0xD000
# end = 0xDFFF
# why = "I/O"

# Extra files for a disk image:
# [[file]]
# path = "level1.bin"
# name = "LEVEL1"
# type = "prg"

[options]
# basic_stub = true              # 10 SYS <entry> at $0801 (default: on when the code does not start at $0801)
# disk_name = "MY GAME"
# disk_id = "01"
# cart_header = true             # .crt: start vectors and CBM80 at $8000
"#;

const SPECTRUM: &str = r#"# rcld layout for the ZX Spectrum.  rcld build this-file.toml
machine = "spectrum"
name = "MYGAME"                  # tape header name (10 characters at most)
entry = "start"
output = "mygame.tap"            # .tap  .tzx  .sna  .z80  .szx  .dsk (+3)  .bin

[[input]]
asm = ["main.asm"]

[[region]]
name = "ram"
start = 0x6000
end = 0xFFFF

[options]
# loader = true                  # tape: BASIC loader (CLEAR, LOAD ""CODE, RANDOMIZE USR entry)
# screen = "loading.scr"         # tape: a 6912 byte screen shown while loading
# pause = 1000                   # tape: pause after each block, milliseconds
# sp = 0xFF00                    # snapshot: stack pointer (default 0)
# border = 0                     # snapshot: border colour
# im = 1                         # snapshot: interrupt mode
# interrupts = false             # snapshot: interrupts enabled at the start
"#;

const CPC: &str = r#"# rcld layout for the Amstrad CPC.  rcld build this-file.toml
machine = "cpc"
name = "MYGAME"
entry = "start"
output = "mygame.dsk"            # .dsk (AMSDOS: RUN"MYGAME.BIN)  .bin

[[input]]
asm = ["main.asm"]

[[region]]
name = "ram"
start = 0x4000
end = 0xA6FF

# [[file]]
# path = "data.bin"
# name = "DATA.BIN"

[options]
# file_name = "GAME.BIN"
"#;

const ATARI_ST: &str = r#"# rcld layout for the Atari ST.  rcld build this-file.toml
machine = "atari-st"
output = "hello.prg"             # a GEMDOS program (.prg, .tos or .ttp); rcld build -o overrides

[[input]]
asm = ["hello.s"]                # assembled together, in this order, as one relocatable 68000 program
                                 # (sections .text .data .bss; the program starts at the first byte of its text)

[define]
# NAME = 1                       # constants for the sources, like rcasm -D
"#;

const PCENGINE: &str = r#"# rcld layout for the PC Engine / TurboGrafx-16 (a HuCard).  rcld build this-file.toml
machine = "pcengine"
name = "MYGAME"
entry = "start"                  # the reset code, in bank 0 ($E000-$FFFF)
output = "mygame.pce"            # .pce or .tg16 (rcld build -o overrides)

[[input]]
asm = ["main.asm"]               # HuC6280 source; `.bank n` then `.org` puts code in 8 KB bank n

# [[input]]
# bin = "graphics.bin"           # a raw binary ...
# bank = 1                       # ... in this bank (default 0)
# at = 0x6000                    # ... at this logical address (only the low 13 bits place it in the bank)

[options]
# size = 262144                  # card size in bytes: default the next power of two (at least 128 KB)

# Vectors: labels irq2, irq1, timer, nmi and reset/start fill $FFF6-$FFFF unless the sources write them.
"#;
