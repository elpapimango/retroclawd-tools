# rcb toolchain: roadmap and phase plans

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (native execution chosen by the owner's "approved, go").
> Each phase below gets its own detailed plan file written when the phase starts (this document is the map); a phase is done when its
> acceptance list passes and its tools and crates are released.

**Goal:** the toolchain of `docs/superpowers/specs/2026-11-23-rcb-toolchain-design.md`: project driver `rcb`, relocatable assembler, linker, dialects,
images for every machine, asset converters, source-level debugging, and tested cores for the new CPUs.

**Architecture:** retroclawd-core holds CPU cores (instruction tables shared by core, disassembler and assembler) and format crates; retroclawd-tools
holds the assembler, linker, archiver, image writers, converters and the `rcb` driver; retroclawd (emulator) reads `.rcdbg` and gets machines later.

**Tech stack:** Rust 2024, `clap`, `serde`, `toml`; no new heavy dependencies; test oracles fetched at test time, never committed.

**Spec:** `docs/superpowers/specs/2026-11-23-rcb-toolchain-design.md`

## Global constraints

- Clean room: cc65 and vasm are references for documented behaviour only; no code, tables or test files copied.
- `#![forbid(unsafe_code)]` in new crates; parsers of untrusted input never panic (fuzz targets).
- Nothing overwrites an existing file without `--force`; writes are atomic; `--json` where the tool lists or reports.
- Every hardware fact is a row in the owning repository's `docs/` fact table with its source.
- Crates publish as `rc-<name>` from retroclawd-core, imported under the short name; tools publish under their bare names.
- Commits end with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.
- Releases: core crates are published in dependency order; the crates.io limit on new crates is 5 at once, then one every 10 minutes.

## Review focus (inputs the spec implies and tests must pin)

1. A source file with 100,000 lines, deeply nested macros or `.include` loops: must stop with an error, never hang or run out of memory.
2. Relocations that overflow (a 16-bit field holding 70000, a branch out of range after linking): linker error naming file, line and symbol.
3. An object, archive, layout, `.cfg` or image file truncated or with hostile sizes: error, no panic, no huge allocation.
4. The same symbol exported by two objects or two archive members: error naming both; an unused archive member never pulled in.
5. CRLF, tabs, a BOM or a missing final newline in sources and layouts.

## Phases

| # | Phase | Repository | Acceptance |
|---|---|---|---|
| 0 | HuC6280 in the tools: `rc-huc6280` published; `rcasm`/`rcdis --cpu huc6280`; `rcld` `machine = "pcengine"` writes `.pce`; test cards built by the tools | core, tools, emulator | Detailed plan: `2026-11-23-phase0-huc6280.md`. Round trip of all 256 opcodes; `pce-tone`, `pce-input`, `pce-timer*` cards built by `rcld` boot in retroclawd and match the manifest goldens |
| 1 | Object model: `.rco`, `.rca`, relocatable `rcasm`, `rcar`, linker over objects; native syntax gains `.segment`, `.import`, `.export`, scopes | tools | Every existing rcasm/rcld test passes unchanged; two-file program with a library archive links; review-focus items 1-4 pinned |
| 2a | Cores: 65816, 2A03 variant, then 68010/20/30 + FPU + PMMU, SPC700, SA-1, Super FX | core | Each core passes its public suite (SingleStepTests where it exists: 65816, SPC700, 68000 family; otherwise differential against the RetroArch libretro cores, black box) |
| 2b | Format crates: iNES/NES 2.0, SFC, X68000 exe and disks, Amiga hunk and ADF | core | Round trip read-write-read; reference files from the owner's RomM read correctly |
| 3 | Image writers: ZX81, Spectrum, CPC, C64, VIC-20, Atari 8-bit first; then Atari ST, Genesis, Mega-CD, Neo Geo, PC Engine, NES, SNES, X68000, Amiga | tools | Each image loads in the matching emulator headless and shows the expected screen (scenario manifests per machine) |
| 4 | Dialects: `ca65` with `.cfg` layouts; then `vasm` `std`, `mot`, `oldstyle` | tools | Fetched public projects assemble; bytes equal the real tool's where installed; unsupported directives error by name |
| 5 | `rcb`: project file, `new/build/run/debug/clean/check/targets/kits`, templates, hardware kits | tools | `rcb new` then `rcb run` boots on every machine the emulator has |
| 6 | Asset converters `rcasset` / `rcb asset` | tools | Golden round trips: picture -> data -> emulator screen |
| 7 | Debug info: `.rcdbg` writer; `rcdd --debug-info`, source stepping | tools, emulator | Break on `file:line`, step by source line on three CPUs |

## Order of execution

0 first, as soon as all 21 core crates and the 8 tools of 0.1 are on crates.io (the publish runs in the background). Then 1 and 2a in turn
(2a is independent of 1: work on whichever the previous result unblocks), 2b, 3 for the machines that exist, 4, 5, 6, 7; the machines
that the emulator does not have yet (NES, SNES, X68000, Amiga) get their images checked against the reference files and the libretro cores until
their machines land.

## What each phase plan contains

The detailed plan for a phase lists files, interfaces, test-first steps and the verification command, as `2026-11-23-phase0-huc6280.md` does.
