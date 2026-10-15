# rctape

```
rctape list      TAPE [--json]
rctape info      TAPE [--json]
rctape validate  TAPE [--json]                  exit status 1 when there are problems
rctape extract   TAPE [BLOCKS] [-d DIR | -o FILE] [--raw] [--force]
rctape cat       TAPE BLOCK [--hex | --basic]
rctape cut       TAPE BLOCKS -o OUT [--lossy] [--force]
rctape rm        TAPE BLOCKS [-o OUT] [--force]
rctape merge     TAPE TAPE... -o OUT [--lossy] [--force]
rctape split     TAPE [-d DIR] [--to tap|tzx] [--lossy] [--force]
rctape cleanup   TAPE [--strip-info] [--fix-checksums] [--pause MS] [--merge-pauses] [--drop-empty] [--all]
                      [-o OUT] [--force]
rctape add       TAPE FILES... [--type code|basic|data] [--name N] [--load A] [--line N] [--pause MS]
                      [-o OUT] [--force]
rctape convert   INPUT OUTPUT [--lossy] [--force]
```

`rm`, `cleanup` and `add` change TAPE in place (the file is replaced atomically); `-o` writes a copy instead. Nothing
else overwrites an existing file without `--force`. Block numbers start at 1, as `list` prints them: `3`, `2-5`,
`4-` (to the end), `-3` (from the start), `1,4-6`, `all`.

## Formats

| Format | Handled how |
|---|---|
| ZX Spectrum `.tzx` | block by block: every block of revision 1.20 is kept as it is, so a file written back is byte-for-byte what was read |
| ZX Spectrum `.tap` | read as standard-speed blocks (pause 1000 ms); written back when the tape holds nothing else |
| Commodore `.t64` | `list`, `info`, `extract` (each program becomes a `.prg` with its load address) |
| Commodore `.tap` | `info` only: version, video standard, pulse count, playing time, pulse range. The recording is a stream of pulses, not blocks; decoding the bits needs the KERNAL's or a turbo loader's timing, and is not done |

The kind is recognised from the content (`ZXTape!`, `C64-TAPE-RAW`), and a `.tap` that is neither is read as a
Spectrum `.tap`.

## Spectrum tapes

`list` prints one line per block: number, kind, data size, and what it is. A ROM header block reads as
`Program: NAME LINE 10 (1234 bytes)` or `Bytes: NAME start 32768, length 6912`, the data block after it as
`Data (N bytes)`, text and message blocks show their text, pauses their length (`Stop the tape` for 0 ms). A block
whose XOR checksum is wrong is flagged `BAD CHECKSUM`.

* `extract` writes the data of the chosen blocks as files. A header is not a file of its own: it names the data
  block after it (`NAME.bas` for a program, `NAME.bin` for bytes, `NAME.arr` for an array); a data block with no
  header becomes `NNN-data.bin`; repeated names get `-2`, `-3`. The flag byte and checksum are left off. `--raw`
  writes every data block whole (headers too) as `NNN-header.bin` / `NNN-data.bin`.
* `cat` prints one block's data: the bytes, a hexdump (`--hex`) or the program as BASIC text (`--basic`, tokens
  expanded, the hidden floating point forms skipped).
* `cut` keeps the chosen blocks in a new tape, `rm` deletes them, `merge` joins tapes in order (the newer format
  revision wins).
* `split` cuts at program boundaries: a header and the data block after it are one piece, information blocks in
  front go with the piece they precede, a pause or group end after it stays with it. Pieces are named
  `NN-NAME.tzx` / `.tap` after the header (`block01` ... for headerless ones).
* `cleanup` does what it is asked, nothing else, and says what it changed: `--strip-info` removes text, message,
  archive, hardware and custom info blocks and glue; `--fix-checksums` recomputes wrong checksums of ROM blocks;
  `--pause MS` sets every pause (a pause of 0, the "stop the tape" marker, stays); `--merge-pauses` joins
  consecutive pause blocks; `--drop-empty` removes data blocks with no data; `--all` is the four without `--pause`.
* `add` appends host files as a header and a data block (`--type code`, the default, with `--load`, or `--type
  basic` with `--line` for the autostart line, the file being a tokenised program; `--type data` writes one block
  with no header). The tape is created if it does not exist. A file is one block, 65533 bytes at most.
* `validate` checks the checksums and that every header is followed by a data block of the length it announces.
* `convert` goes between `.tap` and `.tzx` by the output's extension. A `.tap` can hold standard-speed blocks only:
  anything else is an error naming the block, and with `--lossy` the blocks that carry no data (pauses, text,
  groups) are dropped, said in a note.
* `info` also estimates the playing time from the pulse timings (3.5 MHz T-states, as the TZX document counts); with
  loops, jumps or recordings in the tape it says "at least".

## Checked

`rctape/tests/blocks.rs` writes and reads back one block of every kind the format defines, byte for byte, and checks
the playing time against the emulator's own tape player (`zx-tape`). `tests/emulator.rs` makes a `.tzx` and a `.tap`
(through a `.tzx` and back) and has the Spectrum's ROM loader (`LD-BYTES`) load them in the emulator; it runs when
`RETROCLAWD_BIN`, `RETROCLAWD_MACHINE_PLUGIN_DIRS` and `RC_ROMS` are set (see `CLAUDE.md`) and takes about 40 s,
because the tape plays in real time.

## Not done

* Block-level editing of the contents of a block (re-timing, turbo-block conversion), loops/jumps are kept but not
  followed by `split`, and no `.wav`, `.csw`, `.pzx` or `.voc` input or output.
* A decoder for Commodore `.tap` (KERNAL file headers and data, turbo loaders), and `.t64` writing.
* BASIC tokenizing for `add --type basic` (the file must already be tokenised).
