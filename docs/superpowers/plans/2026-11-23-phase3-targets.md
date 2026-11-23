# Phase 3: assemblers and image writers for the new machines

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** a project for the NES, SNES, Amiga, X68000, Genesis, Neo Geo and Mega-CD goes from sources to the machine's own file with the tools: `rcasm` knows the 65816 and the
SPC700, `rcdis` decodes them, `rcld` writes `.nes`, `.sfc`, Amiga hunk executables and `.adf`, X68000 `.X` and `.xdf`, Genesis `.md`, Neo Geo sets and Mega-CD discs.

**Architecture:** the CPU back ends of `rcasm` build their encoding tables by enumerating the core's disassembler, as for the Z80 and HuC6280. `rcasm`'s addresses become 32-bit
(a 24-bit CPU needs it; the 16-bit CPUs keep their limit through `Cpu::addr_bits`). Each machine is one module of `rcld` with its own layout section, like `pcengine.rs`.

**Spec:** `docs/superpowers/specs/2026-11-23-rcb-toolchain-design.md` (phase 3). **Roadmap:** `2026-11-23-rcb-roadmap.md`. Cores and formats: phase 2 (`rc-w65c816`, `rc-spc700`, `rc-ines`, `rc-sfc`, `rc-amiga`, `rc-x68k`).

## Global constraints

Those of the roadmap. Existing tests of `rcasm`/`rcdis`/`rcld` pass unchanged. A fact not confirmed by an emulator or a datasheet is marked `[H?]` in the docs; images are verified in the emulator
when the machine exists, and by the format crates' round trips until then.

## Review focus

1. 65816 immediates: `lda #$12` is one or two bytes by the accumulator width; `rep #$20` / `sep #$20` change it when tracking is on; `.a8/.a16/.i8/.i16` force it; a stale width is the classic bug, so the assembler says the width in the listing and errors when an immediate does not fit the current width.
2. 65816 address sizes: `lda $12` direct page, `lda $1234` absolute (data bank), `lda $123456` long; `.b/.w/.l` suffixes force; a forward reference is absolute and an error names the fix if it turns out to need long.
3. `MVN`/`MVP` operand order in text (source, destination) versus bytes (destination, source).
4. SPC700 bit addressing: `$12.3` (direct page bit), `$1234.5` (13-bit address and bit number in the top three bits), `/` complement.
5. The 65816 interrupt vectors, SNES header and checksum, NES vector table at `$FFFA`: written from labels (`nmi`, `reset`, `irq`) unless the sources write them, with a note when a label is missing.
6. A 24-bit `.org` in a 16-bit CPU's source is an error naming the CPU.

### Task 1: 32-bit addresses in `rcasm`

**Files:** modify `rcasm/src/asm.rs`, `cpu.rs`, `lib.rs`, `main.rs`, `object.rs`; `rcld` call sites; tests.

- [ ] `Site.addr: u32`, `Segment.start: u32`, `ListEntry.addr: Option<u32>`, `flat_image -> (u32, Vec<u8>)`; `Cpu::addr_bits()` (16 for the 6502, Z80, HuC6280, SPC700; 24 for the 65816) replaces the literal `0xFFFF` checks; listings print the digits the CPU needs.
- [ ] Existing tests pass unchanged (`cargo test --workspace`); a test: `.org $12345` on the 6502 errors "outside 0..65535".

### Task 2: the 65816 back end

**Files:** `rcasm/src/w65816.rs`, `rcasm/src/cpu.rs` (`Cpu::W65816`, names `65816`, `65c816`, `snes`), `rcasm/tests/w65816.rs`.

- [ ] Test: every opcode and each of the four width states round-trips through `w65c816::disassemble::disassemble` (text assembles back to the same bytes), as `tests/huc6280.rs` does.
- [ ] Tests of the documented forms: `lda #$12` / `lda #$1234` by width, `rep #$30` tracks, `lda $12`, `lda $1234`, `lda $123456,x`, `lda ($12),y`, `lda [$12],y`, `lda $12,s`, `lda ($12,s),y`, `jmp ($1234,x)`, `jml [$1234]`, `jsl $123456`, `mvn $7E,$7F` giving bytes `54 7F 7E`, `bra`/`brl`/`per` ranges, `pea $1234`, `.b/.w/.l` suffixes, out-of-range branches named.
- [ ] Implement: table by enumeration per `(m16, x16)`; directives `.a8 .a16 .i8 .i16 .smart [on|off]` (default on); `Site` carries the widths.
- [ ] Objects: relocations for 24-bit fields (width 3) and the 16-bit absolute ones work through the existing probing (test with `.import`).

### Task 3: the SPC700 back end

Same shape: `rcasm/src/spc700.rs`, tests round-tripping every opcode through `spc700::disassemble::disassemble`, bit operands, `dd,ds` and `d,#i` byte orders, `TCALL n`, `PCALL`, branches.

### Task 4: `rcdis` for both

- [ ] `rcdis --cpu 65816 [--m16] [--x16]` follows `REP`/`SEP` in flow mode; `--cpu spc700`; reassembly round trips like `tests/roundtrip.rs`.

### Task 5: NES and SNES in `rcld`

- [ ] `machine = "nes"`: 6502 sources, `[options] mapper, prg_banks, chr` (a `bin`), mirroring, battery; output `.nes` through `rc-ines`; vectors `nmi`/`reset`/`irq` at `$FFFA` of the last PRG bank (fixed bank at `$C000`), `.bank` selects 16 KB banks for mapper code.
- [ ] `machine = "snes"`: 65816 sources placed by `.bank`/`.org` (a 24-bit `.org` gives the bank), `[options] mapping = lorom|hirom|exhirom, fast, title, cart_type, ram_size, country`; the header, vectors from labels (`nmi`, `reset`, `irq`, `cop`, `brk`, `abort`) and the checksum written through `rc-sfc`; output `.sfc`.
- [ ] Tests: built images re-parse with `rc-ines` / `rc-sfc`; the reset vector points at `reset`; overlap and out-of-bank errors name the pieces; an SPC700 program can be included as a `bin` (the SNES audio code).

### Task 6: Amiga and X68000 (68000 sources)

- [ ] `machine = "amiga"`: output `.exe`/no extension hunk file (from the 68000 assembler's text/data/bss and relocations, text -> `HUNK_CODE`, data -> `HUNK_DATA`, bss -> `HUNK_BSS`, each relocation to the hunk its value points into) and `.adf` (a disk with the program, `s/startup-sequence`, extra `[[file]]`s, boot code option) through `rc-amiga`.
- [ ] `machine = "x68000"`: `.X` through `rc-x68k` (relocations as deltas), `.xdf` holding the files raw (the Human68k filesystem is added with the machine).
- [ ] Tests: output re-parses; `Executable::load` of the hunk file equals the 68000 assembler's relocated image; a program with two relocated pointers links correctly.

### Task 7: Genesis, Neo Geo, Mega-CD

- [ ] Genesis `.md`: 68000 flat image at `$000000`, the 256-byte vector/header block (`SEGA MEGA DRIVE`, title, checksum at `$18E`, ROM end, region), padded to a power of two; Z80 sound code as a `bin` placed where the layout says.
- [ ] Neo Geo: the cartridge set the emulator reads (see `machines/snk/docs`): P (68000 program with the `NEO-GEO` header at `$100`), S (fix), C (sprites, odd/even interleaved), M (Z80), V (ADPCM), zipped by name; tests re-read the zip with the emulator's own expectations of file names and sizes.
- [ ] Mega-CD: a data disc (ISO 9660 with `IP` and `SP` in the first sectors, the boot `SEGADISCSYSTEM` header) as `.iso`/`.bin+.cue`, plus audio tracks as raw `bin` inputs; checked by the emulator's disc reader (`machines/sega/crates/formats/disc` rules) once the sources are available to the tools.

### Task 8: finish

- [ ] fmt, clippy, tests green on three OS; docs for each machine (`docs/rcld.md` sections, `docs/rcasm.md` CPU sections); fuzz targets for the new readers; tools 0.3.0 published; roadmap note.

## Self-review

Covers the machine list of the spec's matrix for the new machines. The existing machines (ZX81 `.p` writer, VIC-20, Atari 8-bit `.atr`, Atari ST `.st`) have writers or are covered by
`rcdisk`/`rcsnap`; the ZX81 and VIC-20 targets in `rcld` are phase 5's work (the `rcb` kits), with the same layout mechanism.

## Status (2026-11-23, tools 0.3.0)

Done: Task 1 (32-bit addresses), 2 (65816), 3 (SPC700), 4 (rcdis for both; the 65816 sweep is linear only), 5 (NES, SNES), 6 (Amiga hunk + ADF, X68000 `.X`), and the Genesis half of 7.
Open: Neo Geo cartridge set and Mega-CD disc (need the emulator's set and disc rules read first), the Human68k filesystem for `.xdf` (when the machine exists), object-mode 68000 (separate assembler module).
