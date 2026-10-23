# Credits and references

* Atari ATR/XEX/CAR tools reuse `atari-media` of `retroclawd-atari`, whose
  format facts cite Atari800 7.2.1, revision
  `63a381884175e3cbfc745ff00dfb3d70d928eb50`,
  [`src/binload.c`](https://github.com/atari800/atari800/blob/63a381884175e3cbfc745ff00dfb3d70d928eb50/src/binload.c),
  [`src/afile.h`](https://github.com/atari800/atari800/blob/63a381884175e3cbfc745ff00dfb3d70d928eb50/src/afile.h), and
  [`src/sio.c`](https://github.com/atari800/atari800/blob/63a381884175e3cbfc745ff00dfb3d70d928eb50/src/sio.c).
  Standard CAR headers and checksums follow the format crate's pinned
  [`DOC/cart.txt`](https://github.com/atari800/atari800/blob/63a381884175e3cbfc745ff00dfb3d70d928eb50/DOC/cart.txt)
  and [`src/cartridge_info.c`](https://github.com/atari800/atari800/blob/63a381884175e3cbfc745ff00dfb3d70d928eb50/src/cartridge_info.c)
  facts; serialization stays in that crate.
  No duplicate parser, DOS filesystem, or emulator implementation is added here.

* The instruction sets, mnemonics and encodings come from the retroclawd crates `z80` and `mos6502`
  (`retroclawd-crates`), which cite their own sources (Z80 undocumented-behaviour documentation, ZEXALL / ZEXDOC,
  Klaus Dormann's 6502 functional test, Tom Seddon's NMOS 6502 tests).
* Assembler conventions (labels, `*=`, local labels, `.macro`, `.if`, `-D`, symbol files) follow common practice in
  `ca65`, `ACME`, `64tass`, `sjasmplus` and `pasmo`; none of their code is used.
* `tzxtools` (<https://github.com/shred/tzxtools>, GPL-3.0) is the behavioural reference for the `rctape`
  commands; it is not a dependency and none of its code is copied.
* VICE's monitor label format (`al C:c000 .name`) is written and read for symbol files.
* CP/M directory and allocation layout, the AMSDOS header (checksum over the first 67 bytes) and the +3DOS header
  follow the Amstrad CPC and Spectrum +3 technical documentation (CPCWiki "AMSDOS Header", "CP/M"; the +3 manual's disk
  specification); behaviour was checked against real AMSDOS running in the emulator.
* `c1541` (VICE) is the reference for what a disk tool should do with Commodore images; none of its code is used.
* The TZX block layouts follow the TZX format document, revision 1.20 (<https://worldofspectrum.net/TZXformat.html>);
  the ROM tape header, the XOR checksum and the BASIC token table follow the Spectrum ROM disassembly, and
  playing times were compared with the `zx-tape` player of `retroclawd-crates`. The Commodore `.t64` / `.tap`
  layouts are read through `c64-tape` (`retroclawd-commodore`).
* Snapshot and cartridge layouts are read and written through the format crates of the families (`z80-snapshot`,
  `sna-snapshot`, `szx-snapshot`, `zx81-snapshot`, `cpc-snapshot`, `cpc-cartridge`, `c64-cartridge`), which cite the World of
  Spectrum `.z80` document, the SZX specification, the CPCWiki SNA page and the CRT format of VICE. The `.crt` hardware type
  names follow VICE's cartridge documentation; the ZX81 character set is the one in the ZX81 manual.
* The C64 BASIC line layout (`SYS` token 0x9E, link pointer) and the Spectrum BASIC token numbers and the hidden five-byte
  number form follow the C64 and Spectrum ROM disassemblies; the CBM80 cartridge signature is the one the C64 KERNAL tests
  at $8004. The loaders were run on the real ROMs in the emulator.

## Atari ST FAT12 disks (`rcdisk`, `crates/rcdisk/src/st.rs`, 2026-10-21)

- Microsoft, *FAT: General Overview of On-Disk Format*, v1.03, December 2000,
  https://download.microsoft.com/download/1/6/1/161ba512-40e2-4cc9-843a-923143f3456c/fatgen103.doc (SHA-256
  `b17d66c796d9cd3070adf4ccbc00add5a8b6b6f5491fe8ea949df778fce60172`): BPB fields, FAT12 entries, directory entries,
  short names.
- *Atari ST Internals*, Abacus Software 1985, pp.178-179: the ST's BPB values and the executable boot sector.
