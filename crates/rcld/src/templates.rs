//! Starting layouts, printed by `rcld template <machine>`.

pub fn template(machine: &str) -> Option<&'static str> {
    Some(match machine {
        "c64" => C64,
        "spectrum" => SPECTRUM,
        "cpc" => CPC,
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
