# rcsnap

```
rcsnap info     FILE [--json]
rcsnap convert  INPUT OUTPUT [--version N] [--name NAME] [--force]
rcsnap dump     FILE [-o OUT] [--range START:LEN] [--bank N] [--force]
rcsnap extract  FILE [-d DIR] [--force]
rcsnap create   OUTPUT --load ADDR:FILE... [--pc A] [--sp A] [--border N] [--im N] [--ei] [--force]
```

Nothing overwrites an existing file without `--force`. The kind of a file is decided by its content (`ZXST`,
`MV - SNA`, `C64 CARTRIDGE`, `RIFF`/`AMS!`) and then by its extension. Addresses take `0x8000`, `$8000` or `32768`.

## Formats

| Format | info | dump | convert |
|---|---|---|---|
| ZX Spectrum `.z80` (v1-v3), `.sna` (48K, 128K), `.szx` | yes | yes | into each other |
| ZX81 `.p`, `.81`, `.p81` | yes | yes (addresses from 0x4009) | `.p` / `.81` <-> `.p81` |
| Amstrad CPC `.sna` (v1-v3) | yes | yes | to another version |
| C64 `.crt` | yes | no | plain 8K / 16K cartridges <-> raw `.bin`; `extract` writes the chips |
| CPC Plus / GX4000 `.cpr` | yes | no | raw 16K pages <-> `.cpr`; `extract` writes the pages |

## Spectrum snapshots

`info` shows the machine (48K, 128K / +2, +2A / +3), every register, interrupt state, border, paging, the AY registers
when the file has them, and how much of each RAM bank is non-zero. `convert` goes between `.z80`, `.sna` and `.szx` by
the output's extension and **says what the target cannot keep** (as `rcsnap: note:` lines on standard error):

* `.sna` keeps one interrupt flip-flop (IFF2), has no halted flag, no AY registers and no 128K paging lock, cannot
  hold a +2A/+3 snapshot at all (an error), and a 48K `.sna` keeps the PC on the stack, so the two bytes below SP are
  overwritten with it when the file is written;
* `.z80` has no AY registers (as written by the format crate) and no halted flag;
* `.szx` keeps everything the model has.

`dump` writes what the CPU sees from 0x4000 to 0xFFFF (with the 128K banks 5, 2 and the one paged in at 0xC000), the
`--range START:LEN` part of it, or with `--bank N` one 128K RAM bank. The ROM is not part of a snapshot. Without `-o`
the bytes go to standard output; `rcsnap dump game.z80 -o mem.bin --range 0x8000:0x1000` followed by `rcdis mem.bin --cpu z80 --org 0x8000` disassembles a part of a snapshot.

`create` builds a 48K snapshot: `--load 0x8000:prog.bin` puts a file in memory (repeatable), the PC defaults to the
first load address, SP to 0 (the top of memory), the border to 7, the interrupt mode to 1 and interrupts to disabled
(`--ei` enables them; the snapshot has no ROM state, so only enable them if the program sets up its own interrupt
routine or the system variables). The format is the output's extension.

## ZX81

`info` shows the name of a `.p81` (the ZX81 character set decoded: letters, digits, space and `" £ $ : ? ( ) > < = + -
* / ; , .`), the size, the system variables D_FILE, VARS and E_LINE, and the number of BASIC lines with the first and
last line number. `convert game.p game.p81 --name "MY GAME"` writes a `.p81` (the name defaults to the output file's
name); `.p81` back to `.p` drops the name. The BASIC itself is not listed (the keyword table is not decoded).

## Amstrad CPC `.sna`

`info` shows the model, the registers, the gate array (pens, ROM / RAM configuration), the CRTC registers, the PPI and
the AY registers, and the RAM size. `convert in.sna out.sna --version 1|2|3` rewrites it as another version, noting
what a lower version no longer records.

## Cartridges

* `.crt`: `info` lists the name, the hardware type (the well-known ones by name), the EXROM / GAME lines and the chips
  (type, bank, load address, size). `convert game.bin game.crt` makes a plain cartridge from an 8K image (EXROM low, at
  0x8000) or a 16K image (EXROM and GAME low); other sizes are an error because they need a bank-switching type that a
  raw file cannot name. `convert game.crt game.bin` writes a plain cartridge's ROM; a banked type (Ocean, Magic Desk,
  EasyFlash...) is refused, use `extract` for its chips.
* `.cpr`: `info` lists the 16K pages. `convert game.bin game.cpr` makes pages of a raw image (the last one padded with
  zeros, said in a note), `convert game.cpr game.bin` joins the pages in order and refuses a gap.

## Checked

`tests/formats.rs` takes 48K, 128K and +2A/+3 snapshots through every format and chains of conversions and compares the
results, covers the ZX81 character set and name header, and the raw <-> cartridge conversions. `tests/emulator.rs`
(needs `RETROCLAWD_BIN`, `RETROCLAWD_MACHINE_PLUGIN_DIRS` and `RC_ROMS`) loads snapshots made by `rcsnap create` in all
three formats in the emulator; takes a 48K and a 128K snapshot the emulator itself wrote with `--dump-snapshot`,
converts them to the other formats and loads each again; and starts an 8K `.crt` made from a raw ROM on the C64.

## Not done

* `create` makes 48K snapshots only; no register patching of an existing snapshot (`set`); no `.szx` blocks beyond
  what the format crate models (e.g. Interface 1, Multiface, Timex).
* The CPC Plus `.sna` ASIC state and banked C64 cartridge types have no raw form; `.cpr` has no machine check.
* ZX81 BASIC is not listed; `.p81` names are limited to the characters the ZX81 has.
