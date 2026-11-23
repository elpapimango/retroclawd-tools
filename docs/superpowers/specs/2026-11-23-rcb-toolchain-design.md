# rcb: a project toolchain for every retroclawd machine

Status: draft for approval, 2026-11-23. Repository: retroclawd-tools (with shared crates in retroclawd-core).

## Intent

The owner wants one modern toolchain, written in Rust and with its own identity, that does what cc65 (the `ca65` assembler, `ld65` linker and
`cl65` driver) and vasm do, for every machine retroclawd emulates. The goal as the owner put it: create a project, write source files and assembler
files, assemble them down to a tape, disk or cartridge directly, run the result in retroclawd and debug it there.

Why: the tools that exist today (`rcasm`, `rcld`, `rcdisk`, `rctape`, `rcsnap`) assemble one whole program and write a few image kinds. Real projects,
including the many that already exist for cc65 and vasm, are built from separate source files, libraries, linker configurations and many target
formats. Building those, with a good workflow, is what is missing.

## Goals

1. Assemble existing **ca65** and **vasm** projects as they are (dialects), and keep our own syntax as the default.
2. Real **object files and libraries**: separate assembly, relocations, imports and exports, scopes, archives.
3. A **linker** that reads our TOML layouts and ld65-style `.cfg` files.
4. **Every emulated machine** can be a target, with the image kinds the emulator loads: tape, disk, snapshot and cartridge (matrix below),
   including a Neo Geo ROM set and a Mega-CD disc.
5. **One driver, `rcb`**: `new`, `build`, `run`, `debug`, `clean`, `check`, described by a project file `rcb.toml`.
6. **Source-level debugging** in `rcdd`: breakpoints by `file:line`, stepping by source line, symbols by name.
7. CPUs, each as an assembler/disassembler target **and** a tested emulation core in retroclawd-core (owner's decision, 2026-11-23): 6502 family
   (6502, 65C02, 6510/8502, 2A03 without decimal mode, undocumented opcodes), 65816, SPC700, Z80, HuC6280, and the 68000 family (68000, 68010, 68020, 68030
   with the 68881/68882 FPU and the 68030 PMMU). SNES enhancement chips: SA-1 (a 65816 with its own bus and extras) and Super FX (GSU).
   The emulator machines for NES, SNES, X68000 and Amiga come later and are built on these cores; the video, sound and I/O chips of those machines
   are not part of this work.
8. **Basic asset converters** (owner's decision): `rcb asset` turns pictures into each machine's native graphics data (C64 charsets, sprites and
   bitmaps; Spectrum attribute screens; CPC modes 0-2; Atari 8-bit and ST planes; Genesis, PC Engine and Neo Geo tiles, sprites and palettes) and WAV
   samples into the machines' sample formats (Genesis and Mega-CD PCM, Neo Geo ADPCM-A/B, ST DMA sound). Converters are small, deterministic and
   configured in `rcb.toml` so a build regenerates the data from the source art.

## Non-goals

- No C compiler (decided: assembly only). The object and project formats must not prevent adding a compiler front end later.
- No ARM, PowerPC, x86, 6800/6809 or other vasm CPUs: no planned machine uses them.
- No video, sound or I/O chips of the new machines (PPU, APU, S-PPU, S-DSP, OPM, CRTC, Paula, Agnus...) and no emulator machine for them yet: the owner adds those with each machine.
- No byte-exact copy of cc65's or vasm's object formats; ld65 `.cfg` files are read, ca65 `.o` / `.lib` files and vasm objects are not.
- No emulator inside the toolchain; `rcb run` starts retroclawd.

## Anti-goals (what would make this a failure even if it technically works)

- A Frankenstein of two foreign tools: our syntax, messages, `--json` output, atomic writes and "never overwrite without `--force`" stay the identity.
- A dialect mode that silently accepts something differently from the original tool: unsupported directives are errors with a clear message, never ignored.
- A debug path that works only for one machine.

## Constraints

- Independent project: `retroclawd-tools` releases on its own (crates.io, GitHub releases) and shares cores and formats through `retroclawd-core`
  (`rc-*` crates). The emulator repository must not be needed to build it. `rcdd` integration is the one place the repositories meet (the debug
  file format is specified in this repository; the emulator reads it).
- Clean room: cc65 and vasm are references for behaviour and documented syntax only; no code, tables or test files are copied. Outputs are
  checked against real programs and, where installed, against the real tools as black boxes (never committed).
- Every machine fact (loader formats, header layouts, memory maps) is a row in a `docs/` fact table with its source, as in the emulator repository.
- `#![forbid(unsafe_code)]` in new crates; untrusted inputs (source, objects, archives, images) never panic (fuzz targets).

## Approaches considered

| | What | Trade-off |
|---|---|---|
| **A (chosen)** | Relocatable assembler writing objects, a real linker, dialect front ends over one core, `rcb` on top | Largest rework (today's `rcasm` is whole-program), but the only one that yields libraries and builds cc65-style projects |
| B | Keep whole-program assembly, add dialects and a project file | Least work; no object files or archives, which the owner wants |
| C | Rewrite as a new core | Cleanest, but discards tested encoders (round-trip tests) and delivers nothing until the end |

The owner picked A.

## Design

### Crates (all in retroclawd-tools unless stated)

| Crate | Role |
|---|---|
| `rc-common` | existing: JSON, `.prg`; gains the object, archive and debug-info formats (`rc_common::obj`) |
| `rcasm` | assembler: front ends (`native`, `ca65`, `vasm-mot`, `vasm-old`, `vasm-std`) share one expression, symbol, macro and encoder core; writes `.rco` objects or, as today, flat output |
| `rcld` | linker: `.rco` and `.rca` in, layout (TOML or `.cfg`) in, image out; checks overlap, regions and reserved ranges (today's behaviour) |
| `rcar` (new) | archives: create, list, extract, add (`.rca`) |
| `rcimg` (new library crate, used by `rcld`/`rcb`) | machine image writers (see matrix); reuses `rcdisk`/`rctape`/`rcsnap` formats |
| `rcasset` (new library crate, used by `rcb asset`) | picture and sample converters per machine (palette quantisation, tile cutting, sprite sheets, ADPCM encoders) |
| `rcb` (new) | driver: project file, build graph (incremental by file hash), templates, hardware kits, `run`/`debug` launching retroclawd |
| `rcdis`, `rcdisk`, `rctape`, `rcsnap`, `rcchd` | existing; gain the new CPU and formats where relevant |

The HuC6280 core moves to `retroclawd-core` (`rc-huc6280`) first so `rcasm`, `rcdis` and the emulator share it.

**New in retroclawd-core:** `rc-w65c816` (65816, with the 6502 family variants), `rc-spc700`, `rc-sa1`, `rc-superfx`, `rc-mc68000` extended to
68010/68020/68030 plus `rc-mc68881` and `rc-mc68030-mmu` (FPU and PMMU instruction sets as extensions of the core), the 2A03 variant of `rc-mos6502`,
and format crates `rc-ines`, `rc-sfc`, `rc-x68k-exe`, `rc-x68k-disk` (XDF/DIM/D88/HDF with Human68k FAT), `rc-amiga-hunk`, `rc-amiga-adf`. The
instruction tables live in the core crates and are used by the core, the disassembler and the assembler, so there is one source of truth (as for the Z80).
Cores expose per-cycle bus activity where a test suite checks it; they are tested against public instruction-level suites (SingleStepTests where
they exist for the CPU) and, as black boxes, the libretro cores in the owner's RetroArch install. Suites are fetched, never committed.

### Formats (specified in `docs/formats.md` in this repository)

- `.rco` object: header, sections (named segments with size, alignment, content, type), symbol table (global, local, scope tree), relocations
  (8/16/24/32 bit, low/high byte, PC-relative, 68000 words, bank bytes), import/export lists, per-line debug info (file, line, address range).
- `.rca` archive: members plus an index of exports for fast lookup.
- `.rcdbg` debug file written by `rcld` for a linked image: files, lines to addresses (with bank), symbols with scope, segment map, machine id.
  `rcdd --debug-info FILE` and `rcb debug` read it. Own format, JSON-serialisable for tooling.
- Layouts: today's TOML plus ld65 `.cfg` (MEMORY, SEGMENTS, FEATURES, SYMBOLS, FILES, FORMATS) read and translated to the same internal layout.

### Dialects

- `native` (default): today's syntax extended with segments, scopes, imports and exports.
- `ca65`: segments, `.proc`/`.scope`, `.import`/`.export`/`.global`, `.struct`/`.enum`/`.union`, macros, `.repeat`, `.if*`, cheap locals, `.byte`/`.word`/`.addr`,
  `.res`, `.align`, `.incbin`, `.include`, `.cpu`/`.p02`/`.pc02`, `.smart`, expressions with ca65 operators and functions (`.sizeof`, `.defined`, `.strlen`...).
- `vasm-mot` (68000, Motorola syntax), `vasm-old` (6502 and Z80, old-style), `vasm-std` (generic): section, org, `equ`/`set`, `macro`/`endm`,
  `rept`, `ifd`/`ifnd`, `include`/`incbin`, `xdef`/`xref`, `dc`/`ds`, `even`, `cnop`, local labels.
- A directive or addressing mode the dialect does not support is an error naming it.

### Machine matrix (what `rcb build` can produce; the emulator loads each)

| Machine | CPU | Outputs |
|---|---|---|
| NES / Famicom | 2A03 (6502) | `.nes` iNES and NES 2.0: NROM, MMC1, UxROM, CNROM, MMC3 first; CHR ROM or RAM, battery, mirroring, trainer |
| SNES / Super Famicom | 65816 + SPC700 (+ SA-1, Super FX) | `.sfc`: LoROM, HiROM, ExHiROM, SA-1 and Super FX map modes, header, checksum, SRAM size; `.spc` sound files |
| Sharp X68000 | 68000 / 68030 | Human68k `.X`/`.R` executables, `.XDF`/`.DIM`/`.D88` floppies, `.HDF` hard disk with a Human68k FAT file system |
| Commodore Amiga | 68000 to 68030 (+ FPU) | hunk executables, `.ADF` floppies (boot block, OFS/FFS), `.HDF` hard-disk images |
| ZX81 | Z80 | `.p` program, `.p81` tape |
| ZX Spectrum 48K/128K/+2/+2A/+3 | Z80 | `.tap`/`.tzx` (BASIC loader + code), `.z80`/`.sna`/`.szx` snapshot, `.dsk` (+3) |
| Amstrad CPC 464/664/6128, CPC+/GX4000 | Z80 | `.cdt`/`.tzx`, `.dsk` (AMSDOS), `.sna`, `.cpr` cartridge |
| C64, C128 | 6502/8502 | `.prg`, `.d64`/`.d71`/`.d81`/`.g64`, `.tap`/`.t64`, `.crt`, boot disks |
| VIC-20 | 6502 | `.prg`, `.d64`, `.tap`, `.crt` |
| Atari 8-bit (800/XL/XE/130XE) | 6502 | `.xex`, `.atr` (boot disk), `.car`, `.cas` |
| Atari ST/STF/STFM/Mega ST/STE | 68000 | `.prg` (GEMDOS), `.st`/`.msa` floppy (boot sector and `AUTO` folder), `.stc` cartridge, ACSI hard-disk image |
| Sega Mega Drive / Genesis | 68000 + Z80 | `.md` ROM (header, checksum, vectors, region, SRAM header), SSF2 bank layout for > 4 MB |
| Sega Mega-CD | 68000 x2 + Z80 | CUE/BIN disc: Sega system header, IP, SP, ISO 9660 files; CD-DA tracks |
| SNK Neo Geo | 68000 + Z80 | ROM set zip: P (program), S (fix tiles), C (sprite tiles, odd/even interleave), M (Z80 program), V (ADPCM); header and `NEO-GEO` signature |
| NEC PC Engine / TurboGrafx-16 | HuC6280 | `.pce` HuCard (any size to 2.5 MB, mapping, reset vector, optional header) |

### `rcb` project

```
rcb new NAME --machine c64|spectrum|...   # project from a template: rcb.toml, src/, a hello program, hardware include kit
rcb build [--target NAME] [--release]     # assemble changed files, link, write all configured outputs, write .rcdbg
rcb run   [--target NAME]                 # build, then start retroclawd with the right machine options and media
rcb debug [--target NAME]                 # build, then start rcdd with the debug info loaded
rcb check | clean | targets | kits
```

`rcb.toml` names the machine, sources and dialect per file or glob, include paths, defines, layout (TOML or `.cfg`), outputs (one or more image
kinds), and run options for the emulator. A **hardware kit** per machine (include files with register and system-call equates, startup stubs,
loaders, boot sectors, headers) ships with the tools and is overridable in the project. Kits are written for this project (equates come from
documentation), not copied from cc65's target headers.

### Debugging

`rcdd` gains `--debug-info FILE` (and `rcb debug` passes it): source lines shown with the disassembly, breakpoints and watch points by
`file:line` and by symbol, step over/into by source line, scope-aware symbol lookup. This part is in the emulator repository and is its own
change there, reviewed separately.

## Phases (each ends with tests, docs and a release of the tools)

0a. Cores and formats of the new CPUs in retroclawd-core (can run in parallel with phases 1 onwards): 65816 and 2A03, SPC700, 68010/20/30 with FPU and
    PMMU, SA-1 and Super FX, then the format crates; each core with its tests before the tools use it.
0. HuC6280 in the tools (owner's request, first, after the 0.1 crates are published): move the core to `retroclawd-core` as `rc-huc6280`;
   `rcasm`/`rcdis` get `--cpu huc6280`; `rcld` gets `machine = "pcengine"` and writes `.pce`; the PC Engine test cards are built with the tools instead
   of `build.py`.
1. Object model: `.rco`/`.rca`, relocatable `rcasm` (native syntax), `rcar`, linker over objects with the current TOML layouts; existing outputs unchanged.
2. Image writers for the machines that already have loaders or images (Spectrum, CPC, C64, VIC-20, Atari 8-bit, ZX81), then Atari ST, Genesis, Mega-CD, Neo Geo.
3. `ca65` dialect and `.cfg` layouts; then the vasm dialects.
4. `rcb` driver, templates, hardware kits.
4b. Asset converters (`rcasset`), machine by machine alongside its image writer where possible.
5. Debug info: `.rcdbg` writer; `rcdd` reader and source stepping (emulator repository).

## Testing and verification

- Round trip: every encoder is checked against the matching disassembler (as `rcasm` does now) for all CPUs, including the HuC6280.
- Real programs: a script fetches public sample projects and well-known libraries (never committed, like ROMs); each builds, and byte-identical
  output with the real `ca65`/`ld65`/`vasm` is checked where they are installed.
- Every image kind loads in retroclawd headless and shows the expected screen (scenario manifests in the emulator repository, one per machine).
- Fuzz targets for the source parsers, object, archive, layout and image readers.
- `rcb new` + `rcb run` smoke test on every machine in CI against a released `rcd`.

## Left to the builder

Internal crate boundaries within the list above; the exact binary layout of `.rco`/`.rca`/`.rcdbg` (specified before use in `docs/formats.md`);
the order of machines inside phase 2; hardware-kit contents beyond the minimum (screen, input, sound hello world).

## Calls made without being asked (check these)

- Driver name `rcb`; extensions `.rco`, `.rca`, `.rcdbg`; project file `rcb.toml`.
- Core accuracy: instruction-exact with per-cycle bus access where the public suites check it; no cycle-exact peripheral timing in the cores.
- Order inside the new work: 65816/2A03 (NES, SNES), then 68010-30, then SPC700, SA-1, Super FX, then X68000 and Amiga formats.
- vasm dialects: `std`, `mot`, `oldstyle` only; `madmac` and others are out.
- Phase order (HuC6280, objects, formats, dialects, driver, debug).
- Asset converters are basic by design: no animation or music tools, no editors; formats in, native data out.
- Mega-CD discs include an ISO 9660 file system; Neo Geo sets are zipped as the emulator expects.
